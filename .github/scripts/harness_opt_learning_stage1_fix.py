#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

rpath = ROOT / "crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs"
text = rpath.read_text()
needle = "pub fn select_graph_skill_candidates(\n"
if "#[allow(dead_code)]\npub fn select_graph_skill_candidates(" not in text:
    if needle not in text:
        raise SystemExit("compatibility selector not found")
    text = text.replace(needle, "#[allow(dead_code)]\n" + needle, 1)
needle = "pub fn select_graph_skill_candidates_with_embeddings(\n"
if "#[allow(clippy::too_many_arguments)]\npub fn select_graph_skill_candidates_with_embeddings(" not in text:
    if needle not in text:
        raise SystemExit("embedding selector not found")
    text = text.replace(needle, "#[allow(clippy::too_many_arguments)]\n" + needle, 1)
rpath.write_text(text)

mpath = ROOT / "crates/davinci-coding-agent/src/native_extensions/learning/mod.rs"
text = mpath.read_text()
start = text.find("    pub fn record_skill_usage_outcome(\n")
if start >= 0:
    end_marker = "        self.record_skill_version_outcome(skill, effective)\n    }\n"
    end = text.find(end_marker, start)
    if end < 0:
        raise SystemExit("record_skill_usage_outcome end not found")
    end += len(end_marker)
    text = text[:start] + text[end:]
mpath.write_text(text)
