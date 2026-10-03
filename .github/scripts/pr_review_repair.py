"""Apply and verify reviewed patches on one isolated branch; never invoke a model."""
from pathlib import Path
import gzip
import hashlib
import json
import os
import re
import subprocess
import sys

BRANCH = "fix/pr-integration-review-20261003"
BASE = "c1c3be67013a362b32f2a763feadc9ec74dafd5e"
ROOT = Path.cwd()
OUT = Path(os.environ["RUNNER_TEMP"]) / "pr-review-verification"
OUT.mkdir(parents=True, exist_ok=True)
raw = gzip.decompress(Path(".github/scripts/pr-review-patches.json.gz").read_bytes())
assert hashlib.sha256(raw).hexdigest() == "d94ff498eac53483a0055ebd281b2ae54a1cf978fef088b0106d12dbcf98a805", "patch payload mismatch"
DATA = json.loads(raw)
assert os.environ.get("GITHUB_REF_NAME") == BRANCH, "refusing a different branch"
subprocess.run(["git", "merge-base", "--is-ancestor", BASE, "HEAD"], check=True)


def run(label, command, red=False, timeout=1200):
    print(f"{label}: {' '.join(command)}", flush=True)
    path = OUT / f"{label}.log"
    with path.open("w", encoding="utf8") as log:
        completed = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    text = path.read_text(errors="replace")
    print(text[-18000:], flush=True)
    if red:
        assert completed.returncode != 0, f"{label}: regression unexpectedly passed"
        assert "could not compile" not in text, f"{label}: compilation failure is not a reproduced regression"
        assert "test result: FAILED" in text or re.search(r"# fail [1-9]", text), f"{label}: expected assertion failures"
    else:
        assert completed.returncode == 0, f"{label}: failed with exit {completed.returncode}"


def collect():
    source = {}
    for name in DATA["paths"]:
        path = Path(name)
        if path.is_file():
            source[name] = path.read_text()
    with gzip.open(OUT / "touched-source.json.gz", "wt", encoding="utf8") as stream:
        json.dump(source, stream)
    diff = subprocess.run(["git", "diff", "--binary", BASE], check=True, capture_output=True).stdout
    (OUT / "integrated-fixes.patch").write_bytes(diff)
    (OUT / "revision.txt").write_text(subprocess.check_output(["git", "rev-parse", "HEAD", "HEAD^{tree}"], text=True))
    (OUT / "manifest.json").write_text(json.dumps({"base": BASE, "branch": BRANCH, "paths": DATA["paths"]}, indent=2))


def finalize_green():
    path = Path("crates/davinci-coding-agent/src/main.rs")
    text = path.read_text()
    warning = '''    if agent.provider == "openai-codex" {
        let capability = davinci_ai::fast_capability_for_model(&default_agent_dir(), &agent.model_id);
        if let Err(error) = agent.service_tier.validate_capability(&agent.model_id, capability) {
            eprintln!("{error}. Preference retained; use /fast to disable Fast or choose a supported model.");
        }
    }
'''
    anchor = '    apply_resolved_models(parsed, &mut agent)?;\n    startup_mark("models resolved");\n'
    assert text.count(warning) == 1 and text.count(anchor) == 1
    text = text.replace(warning, "", 1).replace(anchor, anchor + warning, 1)
    path.write_text(text)


phase = sys.argv[1]
if phase in ("red", "green"):
    patch = OUT / f"{phase}.patch"
    patch.write_text(DATA[phase])
    subprocess.run(["git", "apply", "--check", str(patch)], check=True)
    subprocess.run(["git", "apply", str(patch)], check=True)
    if phase == "green":
        finalize_green()
        rust = [name for name in DATA["paths"] if name.endswith(".rs")]
        run("format", ["rustfmt", "--edition", "2021", "--config", "skip_children=true", *rust])
        run("format-check", ["rustfmt", "--edition", "2021", "--config", "skip_children=true", "--check", *rust])
    collect()
elif phase in ("test-red", "test-green"):
    red = phase == "test-red"
    prefix = "red" if red else "green"
    run(f"{prefix}-usage", ["node", "--test", "--test-reporter=tap", "scripts/tests/subscription-usage.test.mjs", "scripts/tests/subscription-usage-review.test.mjs"], red=red)
    run(f"{prefix}-tier", ["cargo", "test", "-p", "davinci-ai", "--locked", "--lib", "review_"], red=red)
    run(f"{prefix}-design-options", ["cargo", "test", "-p", "davinci-coding-agent", "--locked", "--lib", "review_"], red=red)
    run(f"{prefix}-execution", ["cargo", "test", "-p", "davinci-coding-agent", "--locked", "--bin", "davinci", "review_integration_tests"], red=red)
    if not red:
        run("green-dispatch", ["cargo", "test", "-p", "davinci-ai", "--locked", "--test", "fast_dispatch"])
    collect()
elif phase == "test-contracts":
    run("fast-existing", ["cargo", "test", "-p", "davinci-coding-agent", "--locked", "--bin", "davinci", "fast::tests"])
    run("rpc-existing", ["cargo", "test", "-p", "davinci-coding-agent", "--locked", "--bin", "davinci", "rpc_"])
    run("design-contracts", ["cargo", "test", "-p", "davinci-coding-agent", "--locked", "--test", "design_model", "--test", "design_commands", "--test", "design_generation", "--test", "design_security", "--test", "design_controller", "--test", "design_export", "--test", "design_handoff", "--test", "design_store"])
    run("shared-paths", ["cargo", "test", "-p", "davinci-session", "-p", "davinci-sys", "--locked"])
    run("tier-existing", ["cargo", "test", "-p", "davinci-ai", "--locked", "--lib", "service_tier"])
    run("catalog-existing", ["cargo", "test", "-p", "davinci-ai", "--locked", "--lib", "codex_models"])
    collect()
elif phase == "clippy":
    run("clippy", ["cargo", "clippy", "-p", "davinci-coding-agent", "--locked", "--lib", "--bin", "davinci", "--", "-D", "warnings"])
    collect()
elif phase == "publish":
    # Remove this temporary write-enabled executor and its payload from the final
    # source tree. The persistent CI change is a read-only offline Node test step.
    cleanup = [".github/workflows/pr-review-repair.yml", ".github/scripts/pr_review_repair.py", ".github/scripts/pr-review-patches.json.gz"]
    for name in cleanup:
        Path(name).unlink()
    subprocess.run(["git", "config", "user.name", "github-actions[bot]"], check=True)
    subprocess.run(["git", "config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com"], check=True)
    subprocess.run(["git", "add", "--", *DATA["paths"], *cleanup], check=True)
    subprocess.run(["git", "diff", "--cached", "--check"], check=True)
    subprocess.run(["git", "commit", "-m", "fix: reconcile Design, Fast mode, RPC cancellation and usage evidence"], check=True)
    subprocess.run(["git", "push", "origin", f"HEAD:refs/heads/{BRANCH}"], check=True)
    collect()
    (OUT / "verified.txt").write_text("All focused red/green regressions, adjacent offline contracts, and production clippy checks passed before this isolated-branch commit. No live provider requests were made.\n")
elif phase == "collect":
    collect()
else:
    raise SystemExit(f"unknown phase: {phase}")
