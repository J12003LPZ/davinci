"""Print the style runs of one captured frame.

    python scripts/ui/show_styles.py docs/ui/claude-code-reference/ui 03-slash-all [substring]

Each output row is `<row> [fg,bg=…,bold,dim,…]'text' …`. Blank rows are skipped.
"""
import json
import os
import sys

sys.stdout.reconfigure(encoding="utf-8")
folder, frame = sys.argv[1], sys.argv[2]
needle = sys.argv[3] if len(sys.argv) > 3 else None
with open(os.path.join(folder, frame + ".json"), encoding="utf-8") as f:
    rows = json.load(f)
for index, runs in enumerate(rows):
    if not runs:
        continue
    joined = "".join(run["text"] for run in runs)
    if needle and needle not in joined:
        continue
    parts = []
    for run in runs:
        attrs = [run["fg"]]
        if "bg" in run:
            attrs.append("bg=" + run["bg"])
        attrs.extend(key for key in ("bold", "italic", "dim", "strike", "reverse") if run.get(key))
        parts.append(f"[{','.join(attrs)}]{run['text']!r}")
    print(f"{index:02d}", " ".join(parts))
