import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import cache_report


class CacheReportTests(unittest.TestCase):
    def test_missing_usage_is_unavailable_and_groups_use_summed_tokens(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for seq, usage in enumerate(({"input": 20, "cacheRead": 80, "cacheWrite": 0},
                                         {"input": 100, "cacheRead": 100, "cacheWrite": 0}, None), 1):
                (root / f"{seq}-123-logical.json").write_text(json.dumps({"input": []}))
                if usage is not None:
                    (root / f"{seq}-123-usage.json").write_text(json.dumps(usage))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(cache_report.main(directory), 0)
            self.assertIn("all: input unavailable", output.getvalue())
            self.assertIn("first: input 100, cached 80 (80.0%)", output.getvalue())
            self.assertIn("later: input unavailable", output.getvalue())
            self.assertIn("first_request_zero_cached=no", output.getvalue())
            self.assertIn("output=unavailable usage_completeness=unknown", output.getvalue())

    def test_request_fingerprints_cover_body_and_cache_sensitive_components(self):
        body = {
            "input": [{"role": "user", "content": "hello"}],
            "tools": [{"name": "read", "parameters": {"type": "object"}}],
            "instructions": "stable",
        }
        fingerprints = cache_report.request_fingerprints(body, {"input": body["input"]})
        self.assertEqual(set(fingerprints), {"body", "wire", "instructions", "tools", "input"})
        self.assertTrue(all(len(value) == 16 for value in fingerprints.values() if value != "-"))
        reordered = {"tools": body["tools"], "instructions": "stable", "input": body["input"]}
        self.assertEqual(fingerprints["body"], cache_report.request_fingerprints(reordered)["body"])

    def test_first_request_zero_cached_state_is_explicit(self):
        self.assertEqual(cache_report.first_request_cache_state([(100, 0), (100, 50)]), "yes")
        self.assertEqual(cache_report.first_request_cache_state([(100, 1)]), "no")
        self.assertEqual(cache_report.first_request_cache_state([(None, None)]), "unavailable")

    def test_harness_missing_write_count_and_zero_input_are_unknown(self):
        self.assertEqual(cache_report.token_pair({"input": 10, "cacheRead": 0}), (None, None))
        self.assertEqual(cache_report.first_request_cache_state([(0, 0)]), "unavailable")

    def test_harness_cache_diagnosis_includes_model_and_effective_settings(self):
        body = {"model": "fixture", "reasoning": {"effort": "high"}, "input": []}
        self.assertEqual(cache_report.first_break(body, dict(body, model="other"))[0], "model")
        self.assertEqual(cache_report.first_break(body, dict(body, reasoning={"effort": "low"}))[0], "settings.reasoning")

    def test_report_never_prints_capture_content_or_routing_secrets(self):
        secret = "SYNTHETIC_PRIVATE_BEARER_cookie_path_reasoning"
        with tempfile.TemporaryDirectory(prefix=secret) as directory:
            root = Path(directory)
            body = {
                "instructions": secret,
                "tools": [{"name": secret, "parameters": {"description": secret}}],
                "prompt_cache_key": secret,
                "input": [{"type": secret, "content": secret}],
            }
            for seq, content in enumerate((secret, secret + "-changed"), 1):
                request = dict(body, input=[{"type": secret, "content": content}])
                (root / f"{seq}-123-logical.json").write_text(json.dumps(request))
            output = io.StringIO()
            before = [path.read_bytes() for path in sorted(root.iterdir())]
            with contextlib.redirect_stdout(output):
                self.assertEqual(cache_report.main(directory), 0)
            self.assertNotIn(secret, output.getvalue())
            self.assertIn("break=input[0]", output.getvalue())
            self.assertIn("digest=", output.getvalue())
            self.assertEqual(before, [path.read_bytes() for path in sorted(root.iterdir())])
            repeated = io.StringIO()
            with contextlib.redirect_stdout(repeated):
                self.assertEqual(cache_report.main(directory), 0)
            self.assertEqual(output.getvalue(), repeated.getvalue())

    def test_empty_capture_does_not_print_sensitive_directory(self):
        with tempfile.TemporaryDirectory(prefix="SYNTHETIC_PRIVATE_PATH") as directory:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(cache_report.main(directory), 1)
            self.assertNotIn(directory, output.getvalue())
            self.assertNotIn("SYNTHETIC_PRIVATE_PATH", output.getvalue())

    def test_input_summary_does_not_trust_provider_labels(self):
        summary = cache_report.brief({"role": "SYNTHETIC_SECRET", "content": "PRIVATE"})
        self.assertNotIn("SYNTHETIC_SECRET", summary)
        self.assertNotIn("PRIVATE", summary)
        self.assertIn("digest=", summary)
        self.assertIn("bytes=", summary)

    def test_malformed_capture_and_untrusted_filename_fail_without_content(self):
        with tempfile.TemporaryDirectory(prefix="SYNTHETIC_PRIVATE_PATH") as directory:
            root = Path(directory)
            (root / "1-SECRET-logical.json").write_text("{}")
            (root / "2-123-logical.json").write_text("SYNTHETIC_PRIVATE_CONTENT")
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(cache_report.main(directory), 2)
            self.assertNotIn("SECRET", output.getvalue())
            self.assertNotIn("SYNTHETIC_PRIVATE", output.getvalue())
            self.assertIn("invalid capture", output.getvalue())

    def test_offline_continuity_evaluation_preserves_known_usage_and_absent_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            base = {
                "model": "fixture",
                "instructions": "SYNTHETIC_SECRET",
                "tools": [{"name": "read"}, {"name": "grep"}],
                "input": [{"role": "user", "content": "SYNTHETIC_SECRET"}],
            }
            requests = [
                base,
                dict(base, input=base["input"] + [{"role": "assistant", "content": "ok"}]),
                dict(base, tools=list(reversed(base["tools"]))),
                dict(base, instructions="changed"),
            ]
            for seq, body in enumerate(requests, 1):
                (root / f"{seq}-123-logical.json").write_text(json.dumps(body))
            (root / "1-123-usage.json").write_text(json.dumps(
                {"input": 20, "cacheRead": 80, "cacheWrite": 0, "output": 7}))
            (root / "2-123-wire.json").write_text(json.dumps(
                {"previous_response_id": "SYNTHETIC_SECRET", "input": []}))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(cache_report.main(directory), 0)
            lines = output.getvalue().splitlines()
            self.assertIn("break=-", lines[1])
            self.assertIn("output=7 usage_completeness=unknown", lines[1])
            self.assertIn("delta", lines[2])
            self.assertIn("break=-", lines[2])
            self.assertIn("break=tools", lines[3])
            self.assertIn("break=instructions", lines[4])
            self.assertNotIn("SYNTHETIC_SECRET", output.getvalue())
            self.assertIn("all: input unavailable", output.getvalue())


if __name__ == "__main__":
    unittest.main()
