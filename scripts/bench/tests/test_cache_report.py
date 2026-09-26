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
            for seq, usage in enumerate(({"input": 20, "cacheRead": 80},
                                         {"input": 100, "cacheRead": 100}, None), 1):
                (root / f"{seq}-123-logical.json").write_text(json.dumps({"input": []}))
                if usage is not None:
                    (root / f"{seq}-123-usage.json").write_text(json.dumps(usage))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(cache_report.main(directory), 0)
            self.assertIn("all: input unavailable", output.getvalue())
            self.assertIn("first: input 100, cached 80 (80.0%)", output.getvalue())
            self.assertIn("later: input unavailable", output.getvalue())


if __name__ == "__main__":
    unittest.main()
