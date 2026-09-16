#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
path = ROOT / "crates/davinci-coding-agent/src/native_extensions/token_governor.rs"
text = path.read_text()
old = '        assert_eq!(details["tokenGovernor"]["contentKind"], "log");\n'
new = '        assert_eq!(details["tokenGovernor"]["contentKind"], "testOutput");\n'
count = text.count(old)
if count != 1:
    raise SystemExit(f"expected one stale content-kind assertion, found {count}")
path.write_text(text.replace(old, new, 1))
