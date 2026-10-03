"""Content-free extraction from Codex OTLP JSON logs.

Source contract: openai/codex rust-v0.157.1,
codex-rs/otel/src/events/session_telemetry.rs. Connection establishment is
distinct from response dispatch. These records alone do not establish retry
grouping across transport fallback; callers must not equate attempts and turns.
"""
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import threading
import uuid
import hashlib
import hmac
import re
from urllib.parse import urlsplit

EVENTS = frozenset(("codex.api_request", "codex.websocket_request"))
FIELDS = frozenset(("event.name", "attempt", "duration_ms",
                    "http.response.status_code", "success"))


def objects(parent, name):
    if not isinstance(parent, dict):
        raise ValueError("OTLP container must be an object")
    values = parent.get(name, [])
    if not isinstance(values, list) or any(not isinstance(v, dict) for v in values):
        raise ValueError("OTLP collection must contain objects")
    return values


def span_key(record, salt, field="spanId"):
    trace, span = record.get("traceId"), record.get(field)
    if trace is None or span is None or span in ("", "0" * 16):
        return None
    if (not isinstance(trace, str) or not re.fullmatch(r"[a-fA-F0-9]{32}", trace)
            or not isinstance(span, str) or not re.fullmatch(r"[a-fA-F0-9]{16}", span)):
        raise ValueError("invalid OTLP identity")
    return hmac.new(salt, (trace.lower() + span.lower()).encode(), hashlib.sha256).hexdigest()


def sanitize_logs(payload, salt=None):
    """Discard bodies, resource attributes, errors, endpoints and identifiers.

    Duplicate attributes are ambiguous and invalidate the payload. Preserve the
    emission timestamp for downstream duplicate detection without persisting
    account, request, conversation, or trace identifiers.
    """
    result = []
    for resource in objects(payload, "resourceLogs"):
        for scope in objects(resource, "scopeLogs"):
            for record in objects(scope, "logRecords"):
                attributes, seen = {}, set()
                for attribute in objects(record, "attributes"):
                    key = attribute.get("key")
                    if not isinstance(key, str) or key in seen:
                        raise ValueError("invalid or duplicate OTLP attribute")
                    seen.add(key)
                    if key == "endpoint":
                        container = attribute.get("value")
                        if not isinstance(container, dict):
                            raise ValueError("invalid endpoint attribute")
                        value = container.get("stringValue")
                        path = urlsplit(value).path.strip("/") if isinstance(value, str) else ""
                        attributes["endpoint_kind"] = next((kind for kind in
                            ("responses/compact", "responses", "models")
                            if path == kind or path.endswith("/" + kind)), "unknown")
                        continue
                    if key not in FIELDS:
                        continue
                    value = attribute.get("value")
                    if not isinstance(value, dict) or len(value) != 1:
                        raise ValueError("invalid scalar OTLP attribute")
                    kind, scalar = next(iter(value.items()))
                    if kind not in ("stringValue", "intValue", "doubleValue", "boolValue"):
                        raise ValueError("unsupported OTLP attribute type")
                    if type(scalar) not in (str, int, float, bool):
                        raise ValueError("invalid scalar OTLP value")
                    if isinstance(scalar, str) and len(scalar) > 128:
                        raise ValueError("oversized OTLP value")
                    attributes[key] = scalar
                if attributes.get("event.name") not in EVENTS:
                    continue
                stamp = record.get("timeUnixNano")
                if not isinstance(stamp, str) or not stamp.isascii() or not stamp.isdigit():
                    raise ValueError("missing or invalid OTLP timestamp")
                clean = {"timeUnixNano": stamp, **attributes}
                if salt is not None:
                    clean["span"] = span_key(record, salt)
                result.append(clean)
    return result


def sanitize_traces(payload, salt):
    """Retain ancestry with per-run pseudonyms; discard content and resources."""
    result = []
    names = {"run_sampling_request", "model_client.stream_responses_websocket"}
    for resource in objects(payload, "resourceSpans"):
        for scope in objects(resource, "scopeSpans"):
            for span in objects(scope, "spans"):
                key = span_key(span, salt)
                if key is None:
                    raise ValueError("missing span identity")
                warmup = None
                for attr in objects(span, "attributes"):
                    if attr.get("key") == "websocket.warmup":
                        container = attr.get("value")
                        if not isinstance(container, dict):
                            raise ValueError("invalid warmup attribute")
                        value = container.get("boolValue")
                        if warmup is not None or type(value) is not bool:
                            raise ValueError("invalid warmup attribute")
                        warmup = value
                name = span.get("name")
                result.append({"span": key, "parent": span_key(span, salt, "parentSpanId"),
                    "name": name if isinstance(name, str) and name in names else "other",
                    "warmup": warmup})
    return result


def request_metrics(records, spans, errors=0):
    """Join actual request events to sampling ancestors, never count user turns."""
    result = {"logical_requests": None, "requests": None, "provider_attempts": None,
              "prewarm_attempts": None, "request_telemetry_complete": False}
    if errors or not records:
        return result
    unique = {json.dumps(record, sort_keys=True): record for record in records
              if record.get("endpoint_kind") != "models"}
    result["provider_attempts"] = len(unique)
    if not unique:
        return result
    index = {}
    for span in spans:
        key = span["span"]
        if key in index and index[key] != span:
            return result
        index[key] = span
    logical, prewarm = set(), 0
    for record in unique.values():
        key, visited, sampling, warmup = record.get("span"), set(), None, False
        while key in index and key not in visited:
            visited.add(key)
            span = index[key]
            warmup |= span["warmup"] is True
            if sampling is None and span["name"] == "run_sampling_request":
                sampling = key
            key = span["parent"]
        # Only an explicit root completes the chain. Keep walking after finding
        # the nearest sampling span to validate ancestry and classify prewarm.
        if key is not None:
            return result
        if warmup:
            prewarm += 1
        elif sampling is not None:
            logical.add(sampling)
        else:
            return result
    result.update(logical_requests=len(logical), requests=len(logical),
                  prewarm_attempts=prewarm, request_telemetry_complete=True)
    return result


class Collector:
    """One per subprocess; bounded loopback sink retaining sanitized data only."""
    def __init__(self):
        self.records = []
        self.spans = []
        self.salt = uuid.uuid4().bytes
        self.errors = 0
        self.error_categories = {}
        self.path = "/" + uuid.uuid4().hex + "/v1/logs"
        self.trace_path = self.path.removesuffix("logs") + "traces"
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def setup(self):
                self.request.settimeout(2)
                super().setup()

            def log_message(self, *args):
                # HTTP logging can include untrusted paths or request contents.
                return

            def do_POST(self):
                code = 200
                try:
                    self.connection.settimeout(2)
                    length = int(self.headers.get("Content-Length", "-1"))
                    if self.path not in (owner.path, owner.trace_path):
                        raise ValueError("collector path mismatch")
                    if not 0 <= length <= 8388608:
                        raise ValueError("collector body length invalid")
                    if self.headers.get("Content-Encoding"):
                        raise ValueError("collector content encoding unsupported")
                    if self.headers.get_content_type() != "application/json":
                        raise ValueError("collector content type unsupported")
                    raw = self.rfile.read(length)
                    if len(raw) != length:
                        raise ValueError("incomplete collector body")
                    payload = json.loads(raw)
                    if self.path == owner.path:
                        records = sanitize_logs(payload, owner.salt)
                        target = owner.records
                    else:
                        records = sanitize_traces(payload, owner.salt)
                        target = owner.spans
                    if len(target) + len(records) > 100000:
                        raise ValueError("collector record limit")
                    target.extend(records)
                except (ValueError, OSError, RecursionError) as error:
                    owner.errors += 1
                    safe_errors = {"invalid collector request", "incomplete collector body",
                        "collector record limit", "invalid OTLP identity", "missing span identity",
                        "invalid warmup attribute", "collector path mismatch",
                        "collector body length invalid", "collector content encoding unsupported",
                        "collector content type unsupported"}
                    category = str(error) if str(error) in safe_errors else type(error).__name__
                    owner.error_categories[category] = owner.error_categories.get(category, 0) + 1
                    code = 400
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", "2")
                self.end_headers()
                try:
                    self.wfile.write(b"{}")
                except OSError:
                    owner.errors += 1

        self.server = HTTPServer(("127.0.0.1", 0), Handler)
        self.endpoint = f"http://127.0.0.1:{self.server.server_port}{self.path}"
        self.trace_endpoint = f"http://127.0.0.1:{self.server.server_port}{self.trace_path}"
        self.thread = threading.Thread(
            target=self.server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)

    def overrides(self):
        """TOML CLI values verified against Codex rust-v0.157.1 config schema."""
        return ["otel.log_user_prompt=false", "otel.tool_result.max_bytes=0",
                'otel.trace_exporter={otlp-http={endpoint=' + json.dumps(self.trace_endpoint)
                + ',protocol="json"}}', 'otel.metrics_exporter="none"',
                'otel.exporter={otlp-http={endpoint=' + json.dumps(self.endpoint)
                + ',protocol="json"}}']

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
