"""Capture reference frames from a terminal UI (Claude Code or davinci).

Drives the program in a Windows pseudo console (pywinpty), replays its output
on a pyte screen, and writes one plain-text grid and one style-run JSON file
per snapshot. Used to build docs/ui/claude-code-reference/.

    python scripts/ui/capture_tui.py --phase ui --out <dir> --sandbox <dir>
    python scripts/ui/capture_tui.py --phase ui --bin davinci --out <dir> --sandbox <dir>

Phases: ui (no model call), shell (shell mode; Claude Code may answer with a
model turn), turn (one or two model turns; spends usage).

Requires `pip install pywinpty pyte`. Windows only. This is a maintainer tool,
never a test: it starts real programs and may spend model usage.
"""
import argparse
import json
import os
import queue
import re
import shutil
import threading
import time

import pyte
from winpty import PtyProcess

COLS, ROWS = 120, 40
SGR = re.compile(r"\x1b\[([0-9;:]*)m")


def keep_faint(match):
    """pyte drops SGR 2 (faint). Move it to the blink slot so it survives."""
    params = match.group(1).split(";") if match.group(1) else ["0"]
    out, i = [], 0
    while i < len(params):
        p = params[i]
        if p in ("38", "48", "58") and i + 1 < len(params):
            n = 5 if params[i + 1] == "2" else 3
            out.extend(params[i:i + n])
            i += n
            continue
        if p == "2":
            out.append("5")
        elif p == "22":
            out.extend(["22", "25"])
        else:
            out.append(p)
        i += 1
    return "\x1b[" + ";".join(out) + "m"


class Session:
    def __init__(self, argv, cwd, env, out):
        self.out = out
        os.makedirs(out, exist_ok=True)
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.Stream(self.screen)
        self.queue = queue.Queue()
        self.proc = PtyProcess.spawn(argv, cwd=cwd, env=env, dimensions=(ROWS, COLS))
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        while True:
            try:
                self.queue.put(self.proc.read(65536))
            except Exception:
                self.queue.put(None)
                return

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            try:
                data = self.queue.get(timeout=0.05)
            except queue.Empty:
                continue
            if data is None:
                return
            self.stream.feed(SGR.sub(keep_faint, data))

    def text(self):
        return "\n".join(self.screen.display[r].rstrip() for r in range(ROWS))

    def snap(self, name, wait=1.2):
        self.pump(wait)
        rows = []
        for r in range(ROWS):
            line = self.screen.buffer[r]
            runs, current = [], None
            for c in range(COLS):
                ch = line[c]
                style = (ch.fg, ch.bg, ch.bold, ch.italics, ch.blink, ch.strikethrough, ch.reverse)
                if current and current[0] == style:
                    current[1] += ch.data
                else:
                    current = [style, ch.data]
                    runs.append(current)
            row = []
            for (fg, bg, bold, italic, faint, strike, reverse), text in runs:
                if not text.strip() and bg == "default":
                    continue
                run = {"text": text, "fg": fg}
                if bg != "default":
                    run["bg"] = bg
                for key, value in (("bold", bold), ("italic", italic), ("dim", faint),
                                   ("strike", strike), ("reverse", reverse)):
                    if value:
                        run[key] = True
                row.append(run)
            rows.append(row)
        with open(os.path.join(self.out, name + ".txt"), "w", encoding="utf-8") as f:
            f.write(self.text() + "\n")
        with open(os.path.join(self.out, name + ".json"), "w", encoding="utf-8") as f:
            json.dump(rows, f, ensure_ascii=False, separators=(",", ":"))
        print("captured", name)

    def key(self, data, wait=0.4):
        # One write per escape sequence: split writes arrive as a lone Esc.
        self.proc.write(data)
        self.pump(wait)

    def type(self, text, wait=0.6):
        for ch in text:
            self.proc.write(ch)
            self.pump(0.05)
        self.pump(wait)

    def clear(self):
        self.key("\x1b", 0.3)
        self.key("\x15", 0.3)
        for _ in range(30):
            self.key("\x7f", 0.01)
        self.pump(0.5)

    def close(self):
        self.proc.write("\x03")
        self.pump(0.5)
        self.proc.write("\x03")
        self.pump(1)
        try:
            self.proc.terminate(force=True)
        except Exception:
            pass


def ui_phase(s):
    s.type("?"); s.snap("02-shortcuts"); s.clear()
    s.type("/"); s.snap("03-slash-all")
    s.key("\x1b[B", 0.3); s.key("\x1b[B", 0.3); s.snap("04-slash-down2"); s.clear()
    s.type("/comp"); s.snap("05-slash-comp"); s.clear()
    s.type("/mo"); s.snap("06-slash-mo"); s.clear()
    s.type("@"); s.snap("07-at-all"); s.clear()
    s.type("@Car"); s.snap("08-at-car"); s.clear()
    s.type("!"); s.snap("09-bang"); s.type("git status"); s.snap("10-bang-typed"); s.clear()
    s.type("#"); s.snap("11-hash"); s.clear()
    s.type("hello there"); s.snap("12-typed")
    s.key("\x1b\r", 0.4); s.type("second line"); s.snap("13-multiline"); s.clear()
    for n in range(1, 6):
        s.key("\x1b[Z", 0.6); s.snap(f"{13 + n}-mode-{n}")
    for name, command in (("19-model-picker", "/model"), ("21-config", "/config"),
                          ("22-permissions", "/permissions"), ("23-resume", "/resume"),
                          ("24-help", "/help")):
        s.type(command); s.key("\r", 1.5); s.snap(name)
        if name == "19-model-picker":
            s.key("\x1b[B", 0.4); s.snap("20-model-down")
        s.key("\x1b", 1); s.clear()
    s.key("\x1b", 0.3); s.key("\x1b", 0.8); s.snap("25-double-esc"); s.key("\x1b", 0.5)
    s.type("/comp"); s.key("\r", 2.0); s.snap("26-enter-on-comp"); s.key("\x1b", 1); s.clear()


def shell_phase(s):
    s.type("!"); s.type("echo hi"); s.key("\r", 3); s.snap("50-bang-run")
    s.type("!"); s.type("git log --oneline -3"); s.key("\r", 3); s.snap("51-bang-run2")
    s.type("!"); s.type("exit 3"); s.key("\r", 3); s.snap("52-bang-fail")
    s.type("/"); s.key("\x1b[B", 0.3); s.key("\t", 0.8); s.snap("53-tab-on-slash"); s.clear()
    s.type("/mod"); s.key("\t", 0.8); s.snap("54-tab-mod"); s.clear()
    s.type("look at @READ"); s.key("\r", 0.8); s.snap("55-enter-on-at"); s.clear()


def run_turn(s, prompt, prefix):
    s.type(prompt)
    s.key("\r", 0.2)
    approvals = 0
    for i in range(160):
        s.snap(f"{prefix}-turn-{i:03d}", 0.5)
        text = s.text()
        if "Do you want" in text:
            approvals += 1
            s.snap(f"{prefix}-permission-{approvals}", 0.2)
            s.key("\r", 0.6)
        if i > 10 and "esc to interrupt" not in text and "Do you want" not in text:
            s.pump(3)
            if "esc to interrupt" not in s.text():
                break
    s.snap(f"{prefix}-final", 2)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", choices=["ui", "shell", "turn", "idle"], default="ui")
    parser.add_argument("--bin", default=shutil.which("claude") or "claude")
    parser.add_argument("--out", required=True)
    parser.add_argument("--sandbox", required=True, help="a small git repository to run in")
    parser.add_argument("--prompt", action="append", default=[], help="turn phase prompts, in order")
    args = parser.parse_args()

    env = {k: v for k, v in os.environ.items() if not k.upper().startswith("CLAUDE")}
    env.update(COLORTERM="truecolor", TERM="xterm-256color", FORCE_COLOR="3")
    argv = [args.bin]
    if "claude" in os.path.basename(args.bin).lower():
        # Hooks print into the transcript and would pollute the reference.
        argv += ["--settings", '{"disableAllHooks": true}']
    else:
        env.update(PI_OFFLINE="1", DAVINCI_OFFLINE="1",
                   PI_CODING_AGENT_SESSION_DIR=os.path.join(args.out, "sessions"))

    s = Session(argv, args.sandbox, env, args.out)
    s.pump(6)
    s.snap("00-startup")
    if "trust" in s.text().lower():
        s.key("\x1b[B", 0.5)  # Claude Code focuses "No, exit" first
        s.key("\r", 2)
    s.snap("01-welcome", 3)
    if args.phase == "ui":
        ui_phase(s)
    elif args.phase == "shell":
        shell_phase(s)
    elif args.phase == "turn":
        s.key("\x1b[Z", 0.8)  # leave auto mode so approval panels appear
        for index, prompt in enumerate(args.prompt):
            run_turn(s, prompt, str(30 + 10 * index))
    s.close()


if __name__ == "__main__":
    main()
