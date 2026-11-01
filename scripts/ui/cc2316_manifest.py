"""Validate pinned Claude Code 2.1.316 styled-reference metadata offline."""
import json
import re
from pathlib import Path

VERSION = "2.1.316"


def validate(root: Path) -> list[str]:
    root = Path(root)
    errors = []
    try:
        baseline = json.loads((root / "artifacts/data/baseline.json").read_text(encoding="utf-8"))
        entries = json.loads((root / "artifacts/frames/reference/index.json").read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        return [f"missing or invalid reference manifest: {exc}"]
    if baseline.get("version") != VERSION:
        errors.append("baseline version mismatch")
    if not re.fullmatch(r"[0-9a-fA-F]{64}", str(baseline.get("binary_sha256", ""))):
        errors.append("baseline binary SHA-256 invalid")
    if not isinstance(entries, list) or not entries:
        return errors + ["missing reference entries"]
    frame_dir = root / "artifacts/frames/reference"
    for i, entry in enumerate(entries):
        name = entry.get("json") if isinstance(entry, dict) else None
        if not isinstance(name, str) or Path(name).name != name:
            errors.append(f"entry {i}: invalid filename")
            continue
        file = frame_dir / name
        if not file.is_file():
            errors.append(f"entry {i}: missing capture {name}")
            continue
        try:
            captured = json.loads(file.read_text(encoding="utf-8"))
        except (OSError, ValueError) as exc:
            errors.append(f"entry {i}: invalid capture {exc}")
            continue
        if captured.get("target", {}).get("version") != VERSION:
            errors.append(f"entry {i}: version mismatch")
        if captured.get("dimensions") != entry.get("dimensions"):
            errors.append(f"entry {i}: dimensions mismatch")
    return errors


if __name__ == "__main__":
    import sys
    base = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    issues = validate(base)
    for issue in issues:
        print(issue)
    print(f"{len(issues)} manifest issues")
    sys.exit(bool(issues))
