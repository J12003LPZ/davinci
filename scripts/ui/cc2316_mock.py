#!/usr/bin/env python3
"""Loopback Responses-SSE mock for the DaVinci TUI parity replay (task P03).

Binds ``127.0.0.1`` on an ephemeral port only and never forwards anything. Each
POST consumes the next scripted turn (the last turn repeats once the script is
exhausted). Every request is recorded. The event order matches what
``crates/davinci-ai/src/stream_decoder.rs`` accepts for ``openai-responses``:

* text:  created, output_item.added(message), output_text.delta*,
         output_item.done(message), completed(usage)
* tool:  created, output_item.added(function_call), function_call_arguments.delta*,
         function_call_arguments.done, output_item.done(function_call), completed(usage)

A turn with ``hold=True`` stops after its deltas and keeps the stream open until
``release()`` is called or the client disconnects, so interrupt and queue scenarios
can be driven.
"""
from __future__ import annotations

import json
import threading
import time
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TOOLS = ("read", "write", "edit", "bash")
DEFAULT_USAGE = {"input_tokens": 12, "output_tokens": 5, "total_tokens": 17,
                 "input_tokens_details": {"cached_tokens": 0}}


@dataclass
class Turn:
    kind: str                      # "text" | "tool"
    text: str = ""
    tool: str = ""
    arguments: dict = field(default_factory=dict)
    interval: float = 0.0          # seconds between deltas
    chunk: int = 8                 # characters per text delta
    hold: bool = False             # keep the stream open before the end events
    usage: dict = field(default_factory=lambda: dict(DEFAULT_USAGE))


def text_turn(text: str, interval: float = 0.0, chunk: int = 8, hold: bool = False,
              usage: dict | None = None) -> Turn:
    if chunk < 1:
        raise ValueError("chunk must be at least 1")
    return Turn("text", text=text, interval=interval, chunk=chunk, hold=hold,
                usage=dict(usage or DEFAULT_USAGE))


def tool_turn(tool: str, arguments: dict, interval: float = 0.0, hold: bool = False,
              usage: dict | None = None) -> Turn:
    if tool not in TOOLS:
        raise ValueError(f"tool must be one of {TOOLS}, got {tool!r}")
    return Turn("tool", tool=tool, arguments=dict(arguments), interval=interval, hold=hold,
                usage=dict(usage or DEFAULT_USAGE))


def sse(event: dict) -> bytes:
    return f"event: {event['type']}\ndata: {json.dumps(event)}\n\n".encode("utf-8")


def turn_events(turn: Turn, n: int):
    """Return ``(head, tail)`` for one turn.

    ``head`` is a list of ``(event, delay_after)`` sent before any hold; ``tail`` is the
    list of closing events. Pure, so tests can check the order without a socket.
    """
    rid = f"resp_mock_{n}"
    head = [({"type": "response.created", "response": {"id": rid, "status": "in_progress"}}, 0.0)]
    if turn.kind == "text":
        item = {"type": "message", "id": f"msg_{n}", "role": "assistant", "content": []}
        head.append(({"type": "response.output_item.added", "output_index": 0, "item": item}, 0.0))
        for i in range(0, len(turn.text), turn.chunk):
            head.append(({"type": "response.output_text.delta", "output_index": 0,
                          "delta": turn.text[i:i + turn.chunk]}, turn.interval))
        done_item = dict(item, content=[{"type": "output_text", "text": turn.text}])
        output = [done_item]
        tail = [{"type": "response.output_item.done", "output_index": 0, "item": done_item}]
    else:
        args = json.dumps(turn.arguments)
        item = {"type": "function_call", "id": f"fc_{n}", "call_id": f"call_{n}",
                "name": turn.tool, "arguments": ""}
        head.append(({"type": "response.output_item.added", "output_index": 0, "item": item}, 0.0))
        step = max(1, (len(args) + 1) // 2)
        for i in range(0, len(args), step):
            head.append(({"type": "response.function_call_arguments.delta", "output_index": 0,
                          "delta": args[i:i + step]}, turn.interval))
        done_item = dict(item, arguments=args)
        output = [done_item]
        tail = [{"type": "response.function_call_arguments.done", "output_index": 0,
                 "arguments": args},
                {"type": "response.output_item.done", "output_index": 0, "item": done_item}]
    tail.append({"type": "response.completed",
                 "response": {"id": rid, "status": "completed", "output": output,
                              "usage": turn.usage}})
    return head, tail


class MockServer:
    def __init__(self, turns=None):
        self.turns = list(turns) if turns else [text_turn("ok")]
        self.requests: list[dict] = []
        self._lock = threading.Lock()
        self._holds: list[threading.Event] = []   # one event per active hold
        self._closing = threading.Event()
        self.held = 0
        outer = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, format, *args):  # noqa: A002 - silence the request log
                pass

            def do_GET(self):
                outer._record(self, b"")
                self.send_response(404)
                self.send_header("Content-Length", "0")
                self.end_headers()

            def do_POST(self):
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                n = outer._record(self, body)
                outer._serve(self, n)

        self._httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._httpd.daemon_threads = True
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)

    # -- lifecycle -------------------------------------------------------
    @property
    def port(self) -> int:
        return self._httpd.server_address[1]

    @property
    def host(self) -> str:
        return str(self._httpd.server_address[0])

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}/v1"

    def start(self):
        self._thread.start()
        return self

    def stop(self):
        self._closing.set()
        self.release()
        self._httpd.shutdown()
        self._httpd.server_close()

    def __enter__(self):
        return self.start()

    def __exit__(self, *exc):
        self.stop()

    # -- scripting -------------------------------------------------------
    def release(self):
        """Let the turns held right now finish. A hold reached after this call holds again."""
        with self._lock:
            holds = list(self._holds)
        for event in holds:
            event.set()

    def wait_for_requests(self, n: int, timeout: float = 10.0) -> bool:
        end = time.time() + timeout
        while time.time() < end:
            if len(self.post_requests()) >= n:
                return True
            time.sleep(0.02)
        return False

    # -- recording -------------------------------------------------------
    def _record(self, handler, body: bytes) -> int:
        try:
            parsed = json.loads(body) if body else None
        except ValueError:
            parsed = None
        with self._lock:
            self.requests.append({"method": handler.command, "path": handler.path,
                                  "headers": {k.lower(): v for k, v in handler.headers.items()
                                              if k.lower() != "authorization"},
                                  "body": parsed, "t": time.time()})
            return sum(1 for r in self.requests if r["method"] == "POST") - 1

    def post_requests(self) -> list[dict]:
        with self._lock:
            return [r for r in self.requests if r["method"] == "POST"]

    def assert_request_count(self, n: int):
        got = len(self.post_requests())
        if got != n:
            paths = [r["path"] for r in self.post_requests()]
            raise AssertionError(f"expected exactly {n} provider requests, got {got}: {paths}")

    # -- serving ---------------------------------------------------------
    def _serve(self, handler, n: int):
        turn = self.turns[min(n, len(self.turns) - 1)]
        head, tail = turn_events(turn, n)
        try:
            handler.send_response(200)
            handler.send_header("Content-Type", "text/event-stream")
            handler.send_header("Cache-Control", "no-cache")
            handler.send_header("Connection", "close")
            handler.end_headers()
            handler.close_connection = True
            for event, delay in head:
                handler.wfile.write(sse(event))
                handler.wfile.flush()
                if delay:
                    time.sleep(delay)
            if turn.hold:
                release = threading.Event()
                with self._lock:
                    self.held += 1
                    self._holds.append(release)
                try:
                    while not release.wait(0.05):
                        if self._closing.is_set():
                            return
                        # Detect a client that went away (interrupt): a probe write
                        # of an SSE comment fails once the peer is gone.
                        handler.wfile.write(b": hold\n\n")
                        handler.wfile.flush()
                finally:
                    with self._lock:
                        self.held -= 1
                        self._holds.remove(release)
            for event in tail:
                handler.wfile.write(sse(event))
                handler.wfile.flush()
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError, OSError):
            return
