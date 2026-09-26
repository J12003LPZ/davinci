"""Synthetic OTLP records only; no credentials or live model calls."""
import sys
from pathlib import Path
import unittest
import json
import urllib.request
import urllib.error

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from codex_otel import Collector, sanitize_logs, sanitize_traces, request_metrics


def payload(attributes):
    return {"resourceLogs": [{"resource": {"attributes": [
        {"key": "account.email", "value": {"stringValue": "private"}}]},
        "scopeLogs": [{"logRecords": [{"timeUnixNano": "123",
            "body": {"stringValue": "private prompt"},
            "attributes": [{"key": key, "value": {"stringValue": value}}
                           for key, value in attributes.items()]}]}]}]}


class CodexOtelTests(unittest.TestCase):
    def test_malformed_endpoint_and_warmup_are_controlled_errors(self):
        value = payload({"event.name": "codex.api_request", "endpoint": "unused"})
        value["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["attributes"][-1]["value"] = None
        with self.assertRaises(ValueError):
            sanitize_logs(value)
        value = {"resourceSpans": [{"scopeSpans": [{"spans": [{
            "traceId": "a" * 32, "spanId": "1" * 16,
            "attributes": [{"key": "websocket.warmup", "value": None}]}]}]}]}
        with self.assertRaises(ValueError):
            sanitize_traces(value, b"salt")

    def test_trace_metadata_is_private_and_groups_retries(self):
        spans = [
            {"traceId": "a" * 32, "spanId": "1" * 16,
             "name": "run_sampling_request"},
            {"traceId": "a" * 32, "spanId": "2" * 16,
             "parentSpanId": "1" * 16, "name": "stream_request",
             "attributes": [{"key": "prompt", "value": {"stringValue": "private"}}]},
            {"traceId": "a" * 32, "spanId": "3" * 16,
             "name": "model_client.stream_responses_websocket",
             "attributes": [{"key": "websocket.warmup", "value": {"boolValue": True}}]},
        ]
        clean = sanitize_traces({"resourceSpans": [{"scopeSpans": [{"spans": spans}]}]}, b"salt")
        records = []
        for stamp, span in [("123", "2"), ("124", "2"), ("125", "3")]:
            value = payload({"event.name": "codex.websocket_request"})
            record = value["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]
            record.update(timeUnixNano=stamp, traceId="a" * 32, spanId=span * 16)
            records.extend(sanitize_logs(value, b"salt"))
        metrics = request_metrics(records + records[:1], clean)
        self.assertEqual(metrics["logical_requests"], 1)
        self.assertEqual(metrics["provider_attempts"], 3)
        self.assertEqual(metrics["prewarm_attempts"], 1)
        self.assertTrue(metrics["request_telemetry_complete"])
        for private in ("private", "a" * 32, "2" * 16):
            self.assertNotIn(private, json.dumps([clean, records]))

    def test_missing_trace_never_invents_logical_requests(self):
        records = sanitize_logs(payload({"event.name": "codex.websocket_request"}))
        result = request_metrics(records, [])
        self.assertEqual(result["provider_attempts"], 1)
        self.assertIsNone(result["logical_requests"])
        self.assertFalse(result["request_telemetry_complete"])

    def test_conflicting_trace_and_collection_error_invalidate_metrics(self):
        records = sanitize_logs(payload({"event.name": "codex.api_request"}))
        result = request_metrics(records, [], errors=1)
        self.assertIsNone(result["provider_attempts"])

    def test_loopback_collector_and_overrides(self):
        with Collector() as collector:
            request = urllib.request.Request(collector.endpoint,
                json.dumps(payload({"event.name": "codex.websocket_request",
                                    "success": "true"})).encode(),
                {"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=2) as response:
                self.assertEqual(response.status, 200)
            self.assertIn("otel.log_user_prompt=false", collector.overrides())
        self.assertEqual(len(collector.records), 1)
        self.assertEqual(collector.errors, 0)

    def test_bad_json_marks_collection_incomplete(self):
        with Collector() as collector:
            request = urllib.request.Request(collector.endpoint, b"invalid",
                {"Content-Type": "application/json"})
            with self.assertRaises(urllib.error.HTTPError):
                urllib.request.urlopen(request, timeout=2)
        self.assertEqual(collector.records, [])
        self.assertEqual(collector.errors, 1)

    def test_only_allowlisted_request_metadata_survives(self):
        result = sanitize_logs(payload({"event.name": "codex.api_request",
            "attempt": "0", "duration_ms": "12", "error.message": "private",
            "account.email": "private", "endpoint": "https://private/?token=private"}))
        self.assertEqual(result, [{"timeUnixNano": "123", "event.name": "codex.api_request",
                                  "attempt": "0", "duration_ms": "12", "endpoint_kind": "unknown"}])

    def test_catalog_fetch_is_not_model_generation(self):
        records = sanitize_logs(payload({"event.name": "codex.api_request",
            "endpoint": "https://example.invalid/backend-api/codex/models?secret=private"}))
        self.assertEqual(records[0]["endpoint_kind"], "models")
        self.assertNotIn("private", json.dumps(records))
        self.assertEqual(request_metrics(records, [])["provider_attempts"], 0)
        self.assertFalse(request_metrics(records, [])["request_telemetry_complete"])

    def test_connection_events_are_not_inference_requests(self):
        self.assertEqual(sanitize_logs(payload({"event.name": "codex.websocket_connect"})), [])

    def test_malformed_payloads_rejected(self):
        for value in (None, [], {"resourceLogs": {}}, {"resourceLogs": [None]}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                sanitize_logs(value)

    def test_duplicate_attribute_is_rejected(self):
        value = payload({"event.name": "codex.api_request"})
        attrs = value["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["attributes"]
        attrs.append(attrs[0])
        with self.assertRaises(ValueError):
            sanitize_logs(value)
