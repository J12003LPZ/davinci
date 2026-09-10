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
failed: list[str] = []
assert proc.stdout is not None
for line in proc.stdout:
    print(line, end="", flush=True)
    stripped = line.rstrip("\n")
    lines.append(stripped)
    match = re.match(r"^test (.+) \.\.\. FAILED$", stripped)
    if match and match.group(1) not in failed:
        failed.append(match.group(1))

code = proc.wait()
if code:
    if not failed:
        failed.append("cargo test --workspace failed; no FAILED test line was captured")

    for name in failed:
        context = []
        needle = f"test {name} ... FAILED"
        for i, line in enumerate(lines):
            if line == needle:
                context = lines[max(0, i - 3) : min(len(lines), i + 8)]
                break
        message = name
        if context:
            message += " | " + " || ".join(context)
        safe = message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::error title=Rust workspace test failed::{safe}")

sys.exit(code)
