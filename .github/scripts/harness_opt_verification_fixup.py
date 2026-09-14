#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
path = ROOT / "crates/davinci-agent/src/lib.rs"
text = path.read_text()

for signature in [
    "    pub(crate) fn record_successful_mutation(&self) {",
    "    pub(crate) fn record_verification_result(&self, succeeded: bool) {",
]:
    annotated = "    #[allow(dead_code)]\n" + signature
    if annotated in text:
        continue
    if signature not in text:
        raise SystemExit(f"verification compatibility wrapper missing: {signature.strip()}")
    text = text.replace(signature, annotated, 1)

path.write_text(text)
