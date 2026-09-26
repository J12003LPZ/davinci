"""Campaign setup boundaries. No model calls occur in this module."""
import json
from pathlib import Path
import shutil
import os
import signal
import subprocess
import time
import hashlib
import platform
import re
import sys
from datetime import datetime, timezone

from campaign import LEGACY_TASKS, digest, file_hash

BASE_SETTINGS = {"decisionIntelligence": {"enabled": False},
    "compaction": {"enabled": True, "reserveTokens": 16384, "keepRecentTokens": 20000},
    "showCacheMissNotices": False, "transport": "auto", "serviceTier": None,
    "effortPolicy": "fixed", "toolSurface": "full", "autoVerify": True,
    "promptProfile": "stable"}


def stop_reason(stdout, stderr):
    errors = [stderr]
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict):
            continue
        if event.get("type") in ("error", "turn.failed", "auto_retry_start", "auto_retry_end"):
            errors.append(json.dumps(event))
        elif event.get("type") == "message_end":
            message = event.get("message")
            if isinstance(message, dict) and message.get("stopReason") == "error":
                errors.append(str(message.get("errorMessage", "")))
    text = "\n".join(errors).lower()
    if any(value in text for value in ("usage limit", "rate limit", "rate_limit", "quota exceeded", "insufficient_quota")):
        return "usage_limit"
    if any(value in text for value in ("unauthorized", "invalid api key", "incorrect api key",
            "authentication failed", "credentials expired", "refresh token expired", "not logged in")):
        return "credentials"
    return None


def pin_model_store(path, model):
    """Pin one existing public Codex catalog record, without owner configuration."""
    store = json.loads(Path(path).read_text(encoding="utf-8"))
    matches = [entry for entry in store.get("providers", {}).get("openai-codex", {}).get("models", [])
               if entry.get("id") == model]
    if len(matches) != 1:
        raise ValueError("requested model must occur exactly once in the catalog")
    entry = matches[0]
    if "headers" in entry and not entry["headers"]:
        entry = {key: value for key, value in entry.items() if key != "headers"}
    allowed = {"id", "name", "api", "provider", "baseUrl", "reasoning", "input", "cost",
               "contextWindow", "maxTokens", "compat", "thinkingLevelMap"}
    if (set(entry) - allowed or entry.get("provider") != "openai-codex"
            or entry.get("api") != "openai-codex-responses"
            or entry.get("baseUrl") != "https://chatgpt.com/backend-api"):
        raise ValueError("catalog record must use the public Codex route without credentials or overrides")
    return {"providers": {"openai-codex": {"models": [entry]}}}


def model_stop_reason(stdout, expected):
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict) or event.get("type") != "provider_observation":
            continue
        observation = event.get("observation", {})
        if (isinstance(observation, dict) and observation.get("kind") == "logical_start"
                and observation.get("purpose") == "coding"
                and observation.get("model") != "openai-codex/" + expected):
            return "model_mismatch"
    return None


def agent_source():
    explicit = os.environ.get("DAVINCI_CODING_AGENT_DIR", os.environ.get("PI_CODING_AGENT_DIR"))
    if explicit:
        return Path(explicit).expanduser()
    current = Path.home() / ".davinci" / "agent"
    return current if current.exists() else Path.home() / ".pi" / "agent"


def source_identity(root):
    def git_bytes(*args):
        return subprocess.run(["git", *args], cwd=root, capture_output=True, check=True).stdout
    sha = git_bytes("rev-parse", "HEAD").decode().strip()
    dirty = hashlib.sha256(git_bytes("diff", "HEAD", "--binary"))
    # Include new implementation files that are not in git diff yet. Never scan
    # owner configuration or credential directories.
    paths = git_bytes("ls-files", "--others", "--exclude-standard", "-z", "--", "crates", "scripts/bench")
    for name in sorted(paths.decode("utf-8").split("\0")):
        if name:
            dirty.update(name.encode())
            dirty.update(file_hash(Path(root) / name).encode())
    return {"source_sha": sha, "dirty_diff_hash": dirty.hexdigest()}


def checkpoint_identity(executable):
    """Read build provenance bound to copied bytes; never infer it from current HEAD."""
    executable = Path(executable)
    sidecar = executable.with_suffix(executable.suffix + ".identity.json")
    try:
        identity = json.loads(sidecar.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ValueError("missing or invalid checkpoint identity sidecar") from error
    if not isinstance(identity, dict) or identity.get("schema_version") != 1:
        raise ValueError("unsupported checkpoint identity")
    for field, length in (("binary_sha256", 64), ("source_sha", 40), ("dirty_diff_hash", 64)):
        value = identity.get(field)
        if not isinstance(value, str) or re.fullmatch("[0-9a-f]{" + str(length) + "}", value) is None:
            raise ValueError("invalid checkpoint " + field)
    if identity["binary_sha256"] != file_hash(executable):
        raise ValueError("checkpoint binary changed after provenance was recorded")
    return {field: identity[field] for field in ("source_sha", "dirty_diff_hash")}


def campaign_identity(root, variant, fixtures, harnesses, model, effort, settings, repo):
    """Pin executable bytes and configuration before any timed subprocess."""
    executables, identities = {}, {}
    source = source_identity(repo)
    for harness in harnesses:
        selected = os.environ.get("BENCH_" + harness.upper(), harness)
        found = shutil.which(selected)
        if found is None:
            raise ValueError("missing executable: " + harness)
        executable = str(Path(found).resolve())
        if harness == "davinci" and Path(executable).is_relative_to(Path(repo).resolve()):
            raise ValueError("BENCH_DAVINCI must be an immutable copy outside the repository")
        executables[harness] = executable
        version = subprocess.run([executable, "--version"], capture_output=True,
                                 text=True, timeout=30, check=True).stdout.strip()
        identities[harness] = {
            "binary_sha256": file_hash(executable), "version": version,
            **(checkpoint_identity(executable) if harness == "davinci" else {"source_sha": None, "dirty_diff_hash": None}),
            "effective_settings": settings if harness == "davinci" else {
                "ignore_user_config": True, "effort": effort, "service_tier": "default",
                "telemetry": "local-sanitized-otlp-logs-and-traces"}}
    return {"schema_version": 2, "campaign": Path(root).name, "variant": variant,
            "fixture_hash": fixtures["fixture_hash"], "model": model,
            "effort_policy": effort, "service_tier": "default",
            "executables": executables, "identities": identities,
            "agent_dir": str((Path(root) / "_agent").resolve()),
            "os": platform.platform(), "cpu": platform.processor(), "python": sys.version,
            "runner_source": source, "resolved_model_identity": None}


def execute(command, workdir, env, timeout):
    """Time launch through process exit; terminate descendants on timeout."""
    started_at = datetime.now(timezone.utc).isoformat()
    started = time.perf_counter()
    try:
        process = subprocess.Popen(command, cwd=workdir, env=env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace",
            start_new_session=os.name != "nt")
    except OSError as error:
        return {"exit": "launch_error", "stdout": "", "stderr": type(error).__name__,
                "wall_s": time.perf_counter() - started, "started_at": started_at,
                "finished_at": datetime.now(timezone.utc).isoformat()}
    try:
        stdout, stderr = process.communicate(timeout=timeout)
        code = process.returncode
    except subprocess.TimeoutExpired:
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
        else:
            os.killpg(process.pid, signal.SIGKILL)
        if process.poll() is None:
            process.kill()
        stdout, stderr = process.communicate(timeout=10)
        code = "timeout"
    return {"exit": code, "stdout": stdout, "stderr": stderr,
            "wall_s": time.perf_counter() - started, "started_at": started_at,
            "finished_at": datetime.now(timezone.utc).isoformat()}


def create_campaign(root, manifest):
    """Exclusive creation prevents appending new variants to historical rows."""
    root = Path(root)
    root.mkdir(parents=True, exist_ok=False)
    (root / "campaign.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def isolate_settings(source, target, settings):
    """Copy credentials privately, never include their contents or hash in reports."""
    source, target = Path(source), Path(target)
    auth = source / "auth.json"
    if not auth.is_file():
        raise ValueError("credential file is unavailable in the selected agent directory")
    if auth.is_symlink():
        raise ValueError("linked credential files require an explicit resolved source")
    target.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(auth, target / "auth.json")
    (target / "settings.json").write_text(
        json.dumps(settings, indent=2) + "\n", encoding="utf-8")
    return {"directory": str(target.resolve()), "settings": settings}


def controlled_environment(inherited, agent_dir):
    """Use copied credentials and declared settings, not inherited experiment knobs."""
    excluded = {"OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_ORG_ID",
                "OPENAI_PROJECT_ID", "ANTHROPIC_API_KEY", "PYTHONPATH",
                "PYTHONSTARTUP", "PYTHONOPTIMIZE"}
    result = {key: value for key, value in inherited.items()
              if not key.upper().startswith(("DAVINCI_", "PI_", "OTEL_"))
              and key.upper() not in excluded}
    result.update({"DAVINCI_CODING_AGENT_DIR": str(Path(agent_dir).resolve()),
                   "PI_CODING_AGENT_DIR": str(Path(agent_dir).resolve()),
                   "PI_LEARNING_DISABLE_BACKGROUND": "1", "PYTHONUTF8": "1"})
    return result


def select_tasks(task_set, requested, large_manifest=None):
    members = list(LEGACY_TASKS)
    if task_set not in ("legacy", "large", "all"):
        raise ValueError("unknown task set")
    if task_set != "legacy":
        if large_manifest is None:
            raise ValueError("large tasks require a frozen manifest")
        validate_large_manifest(large_manifest)
        large = large_manifest.get("tasks")
        members = large if task_set == "large" else members + large
    tasks = members if not requested or requested == ["all"] else requested
    if not tasks or any(t not in members for t in tasks) or len(set(tasks)) != len(tasks):
        raise ValueError("requested tasks must be unique members of the frozen task set")
    return list(tasks)


def validate_large_manifest(manifest):
    """Validate the frozen large-stratum contract before selecting tasks."""
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        raise ValueError("invalid large fixture manifest")
    if manifest.get("task_set") != "large":
        raise ValueError("large fixture manifest has the wrong task set")
    tasks = manifest.get("tasks")
    if (not isinstance(tasks, list) or not tasks
            or any(not isinstance(task, str) or Path(task).name != task
                   or task in (".", "..") or task in LEGACY_TASKS for task in tasks)
            or len(set(tasks)) != len(tasks)):
        raise ValueError("invalid large task membership")
    fixtures = manifest.get("fixtures")
    if not isinstance(fixtures, dict) or set(fixtures) != set(tasks):
        raise ValueError("large manifest fixtures do not match membership")
    for task in tasks:
        entry = fixtures[task]
        if not isinstance(entry, dict) or entry.get("task_set") != "large":
            raise ValueError("missing large fixture metadata: " + task)
        allowed = entry.get("allowed")
        if (not isinstance(allowed, list) or not allowed
                or any(not isinstance(path, str) or Path(path).name in (".", "..")
                       or Path(path).is_absolute() or ".." in Path(path).parts
                       for path in allowed)):
            raise ValueError("invalid large allowlist: " + task)
        for field in ("public_hash", "reference_solution_hash", "hidden_grader_hash"):
            value = entry.get(field)
            if (not isinstance(value, str) or len(value) != 64
                    or re.fullmatch(r"[0-9a-f]{64}", value) is None):
                raise ValueError("invalid large fixture hash: " + task)
        if not isinstance(entry.get("public_verification"), str) or not entry["public_verification"].strip():
            raise ValueError("missing public verification: " + task)
    return True
