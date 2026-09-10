import re
import subprocess
import sys

proc = subprocess.Popen(
    ["cargo", "test", "--workspace"],
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    text=True,
    bufsize=1,
)
lines: list[str] = []
assert proc.stdout is not None
for line in proc.stdout:
    print(line, end="", flush=True)
    lines.append(line.rstrip("\n"))

code = proc.wait()
if code:
    failed: list[str] = []
    for line in lines:
        match = re.search(r"thread '([^']+)' panicked at", line)
        if match and match.group(1) not in failed:
            failed.append(match.group(1))

    for i, line in enumerate(lines):
        if line.strip() != "failures:":
            continue
        for candidate in lines[i + 1 :]:
            if candidate.startswith("test result:"):
                break
            name = candidate.strip()
            if (
                candidate.startswith("    ")
                and "::" in name
                and " " not in name
                and name not in failed
            ):
                failed.append(name)

    if not failed:
        failed.append("cargo test --workspace failed; inspect job output")

    for name in failed:
        safe = name.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::error title=Rust workspace test failed::{safe}")

sys.exit(code)
