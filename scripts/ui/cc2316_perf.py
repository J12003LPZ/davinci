#!/usr/bin/env python3
"""P20: measure the DaVinci shell through the same replay harness.

Reports medians for every gate the harness can measure: boot and first prompt,
input-to-render, resize-to-render, PageDown and idle CPU/RSS. The harness polls
the raw stream every 10-20 ms, so the plan's 20-50 ms of harness noise applies
to the latency rows; the comparison is against the same harness that measured
the reference, not against a frame-cost model.

Usage:
    python scripts/ui/cc2316_perf.py [--runs 3] [--out PATH]
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))
import cc2316_replay as replay  # noqa: E402

BINARY = REPO / "target" / "debug" / "davinci.exe"
ENV = {"PI_DAVINCI_REFERENCE": "1"}
SETTINGS = {"defaultThinkingLevel": "medium", "autoVerify": False}
ENTER, PAGE_UP, PAGE_DOWN = "\r", "\x1b[5~", "\x1b[6~"
LONG = ("Perf turn. " + "This is a scrolling fixture sentence. " * 16).strip()


def key(text, delay=0):
    return {"type": "key", "text": text, "delay_ms": delay}


def wait(ms):
    return {"type": "wait", "ms": ms}


def wait_for(text, timeout_ms=60000):
    return {"type": "wait_for", "text": text, "timeout_ms": timeout_ms}


def timed_key(text, label, delay=0):
    return {"type": "timed_key", "text": text, "label": label, "delay_ms": delay}


def metrics(label):
    return {"type": "metrics", "label": label, "settle_ms": 2000}


def timed_resize(cols, rows, label):
    return {"type": "timed_resize", "cols": cols, "rows": rows, "label": label,
            "settle_ms": 800}


def run(actions, turns=None, expect=0, env=None):
    out = Path(tempfile.mkdtemp(prefix="cc2316_perf_"))
    return replay.replay(BINARY, {"expect_requests": expect, "wait_ms": 45000,
                                  "actions": actions, "turns": turns or [],
                                  "davinci_settings": SETTINGS},
                         replay.Profile(120, 44, env={**ENV, **(env or {})}),
                         out_dir=out)


def metric(cap, label, field):
    for row in cap.metrics:
        if row.get("label") == label and row.get(field) is not None:
            return row[field]
    return None


def median(values):
    clean = [value for value in values if value is not None]
    return round(statistics.median(clean), 1) if clean else None


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--out", type=Path,
                        default=Path("C:/Users/sergi/Desktop/davinci-cc2316-evidence/perf.json"))
    args = parser.parse_args(argv)
    if not BINARY.is_file():
        print("no binary at", BINARY)
        return 2

    boot, prompt, typed, resized, paged, cpu, rss, resize_bytes, page_bytes = (
        [], [], [], [], [], [], [], [], [])
    internal = []
    for _ in range(args.runs):
        trace = Path(tempfile.mkdtemp(prefix="cc2316_trace_")) / "input.txt"
        cap = run([wait_for("DaVinci v1.1.7"), wait_for("❯"),
                   timed_key("q", "input"), metrics("idle"),
                   timed_resize(100, 30, "resize")],
                  env={"DAVINCI_INPUT_TRACE": str(trace)})
        if trace.is_file():
            samples = []
            for line in trace.read_text(encoding="utf-8", errors="replace").splitlines():
                parts = line.split()
                if len(parts) == 2 and parts[0].isdigit():
                    samples.append(int(parts[0]) / 1000.0)
            if samples:
                internal.append(statistics.median(samples))
        if len(cap.waits) >= 2:
            boot.append(cap.waits[0][1])
            prompt.append(cap.waits[1][1])
        typed.append(metric(cap, "input", "render_ms"))
        resized.append(metric(cap, "resize", "resize_ms"))
        resize_bytes.append(metric(cap, "resize", "output_bytes"))
        cpu.append(metric(cap, "idle", "idle_cpu_pct"))
        rss.append(metric(cap, "idle", "rss_mb"))

        cap = run([key("perf one"), wait(700), key(ENTER, 150), wait(2600),
                   key("perf two"), wait(700), key(ENTER, 150), wait(2600),
                   key(PAGE_UP, 150), wait(600),
                   timed_key(PAGE_DOWN, "page_down")],
                  [{"kind": "text", "text": LONG}, {"kind": "text", "text": LONG}], 2)
        paged.append(metric(cap, "page_down", "render_ms"))
        page_bytes.append(metric(cap, "page_down", "output_bytes"))

    result = {
        "runs": args.runs,
        "environment": {"os": "Windows", "terminal": "ConPTY via pywinpty",
                        "exe": "target/debug/davinci.exe", "cols_rows": [120, 44],
                        "config": "isolated", "setting": "tui.reference on"},
        "gates": {
            "first_prompt_ms": median(prompt),
            "input_to_render_ms": median(typed),
            "input_to_render_internal_ms": median(internal),
            "resize_to_render_ms": median(resized),
            "page_down_ms": median(paged),
            "idle_cpu_pct": median(cpu),
            "idle_rss_mb": median(rss),
        },
        "supporting": {
            "boot_to_banner_ms": median(boot),
            "resize_output_bytes": median(resize_bytes),
            "page_down_output_bytes": median(page_bytes),
        },
        "notes": "Medians of screen-polled latencies; the harness adds 20-50 ms. "
                 "CPU is the shell process; RSS is the process tree.",
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=1), encoding="utf-8")
    print(json.dumps(result, indent=1))
    print("wrote", args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
