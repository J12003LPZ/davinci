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


if __name__ == "__main__":
    unittest.main()
