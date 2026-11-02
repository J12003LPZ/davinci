#!/usr/bin/env python3
"""Replay a scenario against a DaVinci binary and capture styled cells (task P03).

``replay(binary, scenario, profile, out_dir=...) -> Capture`` starts an interactive
``davinci`` in a pseudo console against the loopback mock in ``cc2316_mock``, performs
the scenario actions of ``lean-claude-ui/scripts/scenarios/README.md`` (``key``,
``wait``, ``snapshot``, ``resize``, ``mouse``, ``click_text``, plus DaVinci's
``release``), and renders each snapshot with the reference package's
``styled_xterm.process`` into ``claude-tui-styled-capture/2``.

The reference package is read-only and located through ``CC2316_REF`` (default
``C:/Users/sergi/Desktop/lean-claude-ui``).

Not set, on purpose: ``PI_OFFLINE``. With it the interactive shell answers from a local
stub (``offline_stub_message`` in ``main.rs``) and never contacts a provider, so the
loopback mock would see nothing. Without it, the interactive shell's startup catalog
refresh opened an HTTPS connection to a Cloudflare address about 3 s after launch,
ignoring the proxy variables. ``PI_CATALOG_DRY_RUN=1`` (an existing gate in
``catalog_refresh.rs``) stops that. Network is held to loopback by
``PI_DISABLE_NETWORK=1``, ``PI_CATALOG_DRY_RUN=1``, a refusing proxy listener and TCP
sampling (labelled ``startup``/``mid``/``exit`` samples plus a back-to-back ``watch``
sampler, roughly one sample per 0.7 s).

What the network guard cannot see:

* a connection opened and closed between two samples (about 0.7 s apart);
* a connection made by a process outside the sampled tree (a detached or already
  exited child), or one that bypasses the proxy variables and uses no TCP (UDP, DNS);
* traffic through a system proxy the program reads from the OS instead of the
  environment: it would show as a non-loopback connection only if a sample caught it.
A passing guard therefore means "nothing observed", not "nothing happened".
"""
from __future__ import annotations

import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cc2316_mock  # noqa: E402

PASS, FAIL, SKIP, BLOCKED = "PASS", "FAIL", "SKIP", "BLOCKED"
LOOPBACK = {"127.0.0.1", "::1"}
UNSPECIFIED = {"0.0.0.0", "::", ""}
BASE_ENV_KEYS = ("PATH", "SystemRoot", "WINDIR", "TEMP", "TMP")
ACTIONS = ("key", "wait", "snapshot", "resize", "mouse", "click_text", "release")
SCENARIO_KEYS = ("name", "cols", "rows", "wait_ms", "actions", "graceful", "timeout", "turns",
                 "files", "davinci_settings", "expect_requests")
CLI_ARGS = ["--no-extensions", "--no-skills", "--no-prompt-templates", "--no-mcp",
            "--provider", "local", "--model", "demo"]
REPO_ROOT = Path(__file__).resolve().parents[2]
NAME_RE = re.compile(r"^[A-Za-z0-9_.-]+$")


@dataclass(frozen=True)
class Profile:
    cols: int = 120
    rows: int = 44
    name: str = ""
    env: dict = field(default_factory=dict)

    @staticmethod
    def of(value) -> "Profile":
        if isinstance(value, Profile):
            return value
        if isinstance(value, dict):
            return Profile(**value)
        cols, rows = value
        return Profile(cols, rows)


@dataclass
class Capture:
    status: str
    reason: str = ""
    out_dir: Path | None = None
    snapshots: dict = field(default_factory=dict)   # name -> styled capture json path
    raw_path: Path | None = None                    # full, untrimmed recording
    events_path: Path | None = None
    request_count: int = 0
    proxy_attempts: list = field(default_factory=list)
    connection_samples: list = field(default_factory=list)
    exit: str = ""

    @property
    def ok(self) -> bool:
        return self.status == PASS


def ref_root() -> Path:
    return Path(os.environ.get("CC2316_REF") or "C:/Users/sergi/Desktop/lean-claude-ui")


# --------------------------------------------------------------------- guard

class RefusingProxy:
    """Loopback listener that records and drops every connection."""

    def __init__(self):
        self.attempts: list[dict] = []
        self._sock = socket.socket()
        self._sock.bind(("127.0.0.1", 0))
        self._sock.listen(16)
        self._sock.settimeout(0.2)
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._loop, daemon=True)

    @property
    def port(self) -> int:
        return self._sock.getsockname()[1]

    def start(self):
        self._thread.start()
        return self

    def _loop(self):
        while not self._stop.is_set():
            try:
                conn, peer = self._sock.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            try:
                conn.settimeout(0.2)
                try:
                    first = conn.recv(256).split(b"\r\n")[0].decode("latin-1")
                except OSError:
                    first = ""
                self.attempts.append({"peer": f"{peer[0]}:{peer[1]}", "first_line": first})
            finally:
                conn.close()

    def stop(self):
        self._stop.set()
        self._thread.join(2)
        self._sock.close()


_SAMPLE_PS = r"""
$p=@{}
Get-CimInstance Win32_Process | ForEach-Object { $p[[int]$_.ProcessId]=[int]$_.ParentProcessId }
$ids=@([int]%d); $grow=$true
while($grow){ $grow=$false
  foreach($k in @($p.Keys)){ if(($ids -contains $p[$k]) -and ($ids -notcontains $k)){ $ids+=$k; $grow=$true } } }
$c=@(Get-NetTCPConnection -OwningProcess $ids -ErrorAction SilentlyContinue |
  Select-Object LocalAddress,LocalPort,RemoteAddress,RemotePort,@{n='State';e={[string]$_.State}},OwningProcess)
@{pids=$ids; connections=$c} | ConvertTo-Json -Compress -Depth 4
"""


def sample_connections(pid: int, label: str) -> dict:
    """One TCP sample of ``pid`` and its descendants. ``error`` is set when it failed."""
    out = {"label": label, "t": time.time(), "connections": [], "pids": [], "error": ""}
    try:
        p = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command",
                            _SAMPLE_PS % pid], capture_output=True, text=True, timeout=30)
        if p.returncode != 0 or not p.stdout.strip():
            out["error"] = (p.stderr or "no output").strip()[:300]
            return out
        data = json.loads(p.stdout)
        conns = data.get("connections")
        if isinstance(conns, dict):
            conns = [conns]
        pids = data.get("pids")
        out["connections"] = conns or []
        out["pids"] = pids if isinstance(pids, list) else [pids]
    except Exception as exc:  # noqa: BLE001 - the guard must report, never raise
        out["error"] = f"{type(exc).__name__}: {exc}"[:300]
    return out


def _watch(pid: int, stop: threading.Event, into: list):
    """Sample back to back until stopped; each sample takes about a second."""
    while not stop.is_set():
        into.append(sample_connections(pid, "watch"))


def foreign_connections(sample: dict) -> list[dict]:
    """Live connections whose remote end is not loopback. Listeners are ignored."""
    bad = []
    for c in sample["connections"]:
        if c.get("State") in ("Listen", "Bound"):
            continue
        if c.get("RemoteAddress") in LOOPBACK or c.get("RemoteAddress") in UNSPECIFIED:
            continue
        bad.append(c)
    return bad


# ------------------------------------------------------------------- helpers

def check_scenario(scenario) -> str:
    """Return a reason the fixture cannot run this scenario, or '' when it can."""
    if not isinstance(scenario, dict):
        return "scenario must be a dict"
    for key, value in scenario.items():
        if key in SCENARIO_KEYS:
            continue
        if key in ("args", "env") and not value:
            continue
        return f"scenario key {key!r} is not supported by the DaVinci fixture"
    seen = set()
    for i, action in enumerate(scenario.get("actions", [])):
        kind = action.get("type") if isinstance(action, dict) else None
        if kind not in ACTIONS:
            return f"action {i} has unknown type {kind!r}"
        if kind == "snapshot":
            name = action.get("name")
            if not isinstance(name, str) or not NAME_RE.match(name) or name in seen:
                return f"action {i}: snapshot needs a unique name of letters, digits, _ . -"
            seen.add(name)
    for rel in (scenario.get("files") or {}):
        if Path(rel).is_absolute() or ".." in Path(rel).parts:
            return f"file {rel!r} must be a relative path inside the working directory"
    expected = scenario.get("expect_requests")
    counted = isinstance(expected, int) and not isinstance(expected, bool) and expected >= 0
    if scenario.get("turns"):
        if not counted:
            return "a scenario with turns must state expect_requests as an integer (the mock repeats its last turn)"
    elif expected not in (None, 0) or (expected is not None and not counted):
        return "a scenario without turns expects exactly 0 provider requests"
    return ""


def expected_requests(scenario) -> int:
    """The exact provider request count a scenario must produce."""
    return scenario["expect_requests"] if scenario.get("turns") else 0


def inside_repo(path) -> bool:
    """True when ``path`` resolves to the repository root found from this file, or below it."""
    return Path(path).resolve().is_relative_to(REPO_ROOT)


def make_turns(spec) -> list:
    turns = []
    for t in spec or []:
        if t.get("kind") == "tool":
            turns.append(cc2316_mock.tool_turn(t["tool"], t.get("arguments", {}),
                                               interval=t.get("interval", 0.0),
                                               hold=t.get("hold", False)))
        else:
            turns.append(cc2316_mock.text_turn(t.get("text", "ok"), interval=t.get("interval", 0.0),
                                               chunk=t.get("chunk", 8), hold=t.get("hold", False)))
    return turns


def build_env(home: Path, agent: Path, proxy_port: int, extra: dict) -> dict:
    env = {k: os.environ[k] for k in BASE_ENV_KEYS if k in os.environ}
    proxy = f"http://127.0.0.1:{proxy_port}"
    env.update({
        "PI_DISABLE_NETWORK": "1", "PI_SKIP_VERSION_CHECK": "1", "PI_CATALOG_DRY_RUN": "1",
        "PI_LEARNING_DISABLE_BACKGROUND": "1", "PI_HOOKS_DRY_RUN": "1",
        "TERM": "xterm-256color", "COLORTERM": "truecolor",
        "HOME": str(home), "USERPROFILE": str(home),
        "PI_CODING_AGENT_DIR": str(agent), "DAVINCI_CODING_AGENT_DIR": str(agent),
        "HTTP_PROXY": proxy, "HTTPS_PROXY": proxy, "ALL_PROXY": proxy, "NO_PROXY": "127.0.0.1",
    })
    env.update(extra)
    return env


def load_reference():
    """Import the read-only reference package lazily. Returns (module_harness, styled_xterm)."""
    scripts = ref_root() / "scripts"
    if not (scripts / "styled_xterm.py").is_file():
        raise RuntimeError(f"reference package not found at {scripts} (set CC2316_REF)")
    if not (scripts / "node_modules").is_dir() and not os.environ.get("XTERM_PATH"):
        raise RuntimeError("reference package has no node_modules (@xterm/headless)")
    if shutil.which("node") is None:
        raise RuntimeError("node is not on PATH")
    sys.path.insert(0, str(scripts))
    previous = sys.dont_write_bytecode
    sys.dont_write_bytecode = True      # the reference package is read-only
    try:
        import tui_harness
        import styled_xterm
    finally:
        sys.dont_write_bytecode = previous
        sys.path.remove(str(scripts))
    return tui_harness, styled_xterm


def make_harness_class(tui_harness, styled_xterm, scratch: Path):
    """The package's Harness with ``display()`` drawn by xterm.js instead of pyte.

    ``Harness.display`` feeds pyte, which cannot follow DaVinci's output (the package
    itself notes pyte misreads some sequences); it stayed blank here. Everything else,
    including ``find_text``/``click_text`` built on ``display``, is reused unchanged.
    Wide glyphs count as one column when locating text, so a click on a row that has
    them can land a column off.
    """
    class DaVinciHarness(tui_harness.Harness):
        def display(self) -> str:
            raw = styled_xterm.trim_to_current_screen(
                styled_xterm.styled_capture.sanitize(self.text()))
            path = scratch / "_display.ansi"
            path.write_text(raw, encoding="utf-8", newline="")
            dump = styled_xterm.dump(str(path), self.cols, self.rows)
            lines = []
            for y, row in enumerate(dump["rows"]):
                cells = [" "] * self.cols
                for run in row["runs"]:
                    x = run["x"]
                    for ch in run["text"]:
                        if x < self.cols:
                            cells[x] = ch
                        x += 1
                lines.append(f"{y:02d}|" + "".join(cells).rstrip())
            return "\n".join(lines)

    return DaVinciHarness


def kill_tree(pid: int):
    subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], capture_output=True, timeout=30)


def pid_alive(pid: int) -> bool:
    p = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH", "/FO", "CSV"],
                       capture_output=True, text=True, timeout=30)
    return f'"{pid}"' in p.stdout


def relabel(json_path: Path, binary: str):
    """Overwrite the Claude Code labels styled_xterm hardcodes."""
    data = json.loads(json_path.read_text(encoding="utf-8"))
    data["target"] = {"name": "DaVinci", "binary": os.path.basename(binary), "platform": sys.platform}
    json_path.write_text(json.dumps(data, indent=1, ensure_ascii=False), encoding="utf-8")
    html = json_path.with_suffix(".html")
    if html.is_file():
        text = html.read_text(encoding="utf-8")
        text = re.sub(r"Claude Code [0-9.]+", "DaVinci", text)
        html.write_text(text, encoding="utf-8")


# -------------------------------------------------------------------- replay

def replay(binary, scenario, profile, *, out_dir) -> Capture:
    """Run ``scenario`` against ``binary``. ``out_dir`` must be outside the repository."""
    profile = Profile.of(profile)
    out = Path(out_dir)
    cap = Capture(BLOCKED, out_dir=out)
    reason = check_scenario(scenario)
    if reason:
        cap.reason = reason
        return cap
    if inside_repo(out):
        cap.reason = f"out_dir {out} is inside the repository; captures must be written outside it"
        return cap
    if not Path(binary).is_file():
        cap.reason = f"binary not found: {binary}"
        return cap
    if sys.platform != "win32":
        cap.reason = "the replay needs Windows (pywinpty, Get-NetTCPConnection)"
        return cap
    try:
        import winpty  # noqa: F401
    except ImportError:
        cap.reason = "pywinpty is not installed"
        return cap
    try:
        tui_harness, styled_xterm = load_reference()
    except Exception as exc:  # noqa: BLE001
        cap.reason = str(exc)
        return cap
    out.mkdir(parents=True, exist_ok=True)

    root = Path(tempfile.mkdtemp(prefix="cc2316_run_"))
    home = root / "home"
    work = home / "work"      # inside the isolated home so the trust search stops there
    agent = root / "agent"
    for d in (work, agent):
        d.mkdir(parents=True)
    for rel, content in (scenario.get("files") or {}).items():
        target = work / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")

    mock = cc2316_mock.MockServer(make_turns(scenario.get("turns"))).start()
    proxy = RefusingProxy().start()
    settings = {"processManager": {"enabled": False}}
    settings.update(scenario.get("davinci_settings") or {})
    (agent / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
    (agent / "models.json").write_text(json.dumps({"providers": {"local": {
        "baseUrl": mock.base_url, "api": "openai-responses", "apiKey": "sk-test",
        "models": [{"id": "demo", "name": "Demo"}]}}}), encoding="utf-8")

    # The profile sets the size; a scenario's own cols/rows are ignored.
    scratch = root / "scratch"
    scratch.mkdir()
    harness_class = make_harness_class(tui_harness, styled_xterm, scratch)
    harness = harness_class(profile.cols, profile.rows, str(work),
                            build_env(home, agent, proxy.port, profile.env),
                            str(binary), CLI_ARGS)
    timer = None
    raw_path = out / "full.raw"
    events_path = out / "events.jsonl"
    problems: list[str] = []
    timed_out = threading.Event()
    pid = None
    watch_stop, watched, watcher = threading.Event(), [], None
    try:
        harness.start()
        pid = harness.proc.pid
        timer = threading.Timer(float(scenario.get("timeout", 120)),
                                lambda: (timed_out.set(), kill_tree(pid)))
        timer.daemon = True
        timer.start()
        watcher = threading.Thread(target=_watch, args=(pid, watch_stop, watched), daemon=True)
        watcher.start()
        time.sleep(1.0)
        cap.connection_samples.append(sample_connections(pid, "startup"))
        _settle(harness, float(scenario.get("wait_ms", 6000)) / 1000.0)
        actions = scenario.get("actions", [])
        mid = len(actions) // 2
        for i, a in enumerate(actions):
            if i == mid:
                cap.connection_samples.append(sample_connections(pid, "mid"))
            kind = a["type"]
            if kind == "wait":
                harness.wait(a.get("ms", 500))
            elif kind == "key":
                harness.wait(a.get("delay_ms", 0))
                harness.key(a.get("text", ""))
            elif kind == "resize":
                harness.resize(a["cols"], a["rows"])
                harness.wait(a.get("settle_ms", 800))
            elif kind == "mouse":
                harness.mouse(a.get("action", "left"), a.get("x", 10), a.get("y", 10),
                              a.get("button", 0))
            elif kind == "click_text":
                if not harness.click_text(a["text"], a.get("dx", 0)):
                    cap.reason = f"action {i}: text {a['text']!r} is not on the screen"
                    cap.status = BLOCKED
                    break
                harness.wait(a.get("settle_ms", 500))
            elif kind == "release":
                mock.release()
            elif kind == "snapshot":
                _snapshot(harness, styled_xterm, out, a["name"], binary, cap)
        else:
            cap.status = PASS
        if not any(s["label"] == "mid" for s in cap.connection_samples):
            cap.connection_samples.append(sample_connections(pid, "mid"))
        cap.connection_samples.append(sample_connections(pid, "exit"))
    except Exception as exc:  # noqa: BLE001
        cap.status, cap.reason = FAIL, f"{type(exc).__name__}: {exc}"
    finally:
        cap.exit = _end(harness, pid, bool(scenario.get("graceful", True)), problems)
        if timer is not None:
            timer.cancel()
        watch_stop.set()
        if watcher is not None:
            watcher.join(35)
        cap.connection_samples = sorted(cap.connection_samples + watched, key=lambda s: s["t"])
        cap.request_count = len(mock.post_requests())
        mock.stop()
        proxy.stop()
        cap.proxy_attempts = list(proxy.attempts)
        raw_path.write_bytes(harness.raw())
        with events_path.open("w", encoding="utf-8") as f:
            for e in harness.events:
                f.write(json.dumps(e, ensure_ascii=False) + "\n")
        cap.raw_path, cap.events_path = raw_path, events_path
        shutil.rmtree(root, ignore_errors=True)

    if timed_out.is_set():
        problems.append("the scenario timed out and the process tree was killed")
    if cap.proxy_attempts:
        problems.append(f"{len(cap.proxy_attempts)} proxy connection attempt(s): {cap.proxy_attempts[:3]}")
    for s in cap.connection_samples:
        if s["error"]:
            if cap.status == PASS:
                cap.status, cap.reason = BLOCKED, f"TCP sample {s['label']!r} failed: {s['error']}"
        for c in foreign_connections(s):
            problems.append(f"non-loopback connection at {s['label']}: "
                            f"{c.get('RemoteAddress')}:{c.get('RemotePort')} {c.get('State')}")
    expected = expected_requests(scenario)
    if cap.request_count != expected:
        problems.append(f"expected exactly {expected} provider requests, got {cap.request_count}")
    if problems:
        cap.status = FAIL
        cap.reason = "; ".join(problems + ([cap.reason] if cap.reason else []))
    (out / "meta.json").write_text(json.dumps({
        "target": {"name": "DaVinci", "binary": os.path.basename(str(binary)), "platform": sys.platform},
        "profile": {"cols": profile.cols, "rows": profile.rows, "name": profile.name},
        "status": cap.status, "reason": cap.reason, "exit": cap.exit,
        "request_count": cap.request_count, "proxy_attempts": cap.proxy_attempts,
        "connection_samples": cap.connection_samples}, indent=1), encoding="utf-8")
    return cap


def _without_mark(display: str) -> str:
    """The display with the idle mark's box (rows 1-5, columns 0-19) blanked.

    The header mark animates on its own and never stops, so it must not decide whether
    the screen has settled.
    """
    out = []
    for line in display.splitlines():
        y, body = int(line[:2]), line[3:]
        out.append(" " * 20 + body[20:] if 1 <= y <= 5 else body)
    return "\n".join(out)


def _settle(harness, max_seconds: float, gap: float = 0.5):
    """Wait until the screen has text and two reads ``gap`` seconds apart match.

    The shell keeps repainting after it has drawn (output never goes quiet), so the
    check compares the replayed screen text, not the byte stream.
    """
    end = time.time() + max_seconds
    previous = None
    while time.time() < end:
        text = _without_mark(harness.display())
        if previous is not None and text == previous and text.strip():
            return
        previous = text
        time.sleep(gap)
    raise RuntimeError(f"the screen did not settle within {max_seconds:.0f} s")


def _snapshot(harness, styled_xterm, out: Path, name: str, binary, cap: Capture):
    raw = out / f"{name}.full.raw"
    raw.write_bytes(harness.raw())
    harness.event("snapshot", {"name": name, "cols": harness.cols, "rows": harness.rows})
    styled_xterm.process(str(raw), str(out), name, harness.cols, harness.rows)
    path = out / f"{name}.json"
    relabel(path, str(binary))
    cap.snapshots[name] = path


def _end(harness, pid, graceful: bool, problems: list) -> str:
    """End the process and its tree. Returns how it ended."""
    how = "never started"
    if pid is None:
        return how
    how = "killed"
    if graceful:
        try:
            harness.proc.write("\x04")
            for _ in range(40):
                if not harness.proc.isalive():
                    how = "exited"
                    break
                time.sleep(0.1)
        except Exception:  # noqa: BLE001
            pass
    harness.alive = False
    kill_tree(pid)
    try:
        harness.proc.terminate(force=True)
    except Exception:  # noqa: BLE001
        pass
    time.sleep(0.3)
    if pid_alive(pid):
        kill_tree(pid)
        time.sleep(0.5)
        if pid_alive(pid):
            problems.append(f"process {pid} is still running after the kill")
    return how
