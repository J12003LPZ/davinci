#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
stage = (ROOT / ".harness-opt-stage").read_text().strip()


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


if stage == "governor":
    path = ROOT / "crates/davinci-coding-agent/src/runtime_host.rs"
    text = path.read_text()
    text = replace_once(
        text,
        "    /// Exempts `memory_search`, `retrieve_output`, and error outputs.\n",
        "    /// Exempts `memory_search` and `retrieve_output`; large compressible failures remain reversible.\n",
        "runtime host governor comment",
    )
    text = replace_once(
        text,
        "        // memory_search, retrieve_output, and error outputs are strictly exempt\n        if result.is_error || name == \"memory_search\" || name == \"retrieve_output\" {\n",
        "        // Recovery and memory tools stay verbatim; error status alone does not bypass reversible compression.\n        if name == \"memory_search\" || name == \"retrieve_output\" {\n",
        "runtime host error bypass",
    )
    text = replace_once(
        text,
        "        let gov = match self.governor.lock() {\n            Ok(g) => g,\n            Err(p) => p.into_inner(),\n        };\n        gov.retrieve(args)\n",
        "        let mut gov = match self.governor.lock() {\n            Ok(g) => g,\n            Err(p) => p.into_inner(),\n        };\n        gov.retrieve(args)\n",
        "runtime host mutable retrieve",
    )
    path.write_text(text)
