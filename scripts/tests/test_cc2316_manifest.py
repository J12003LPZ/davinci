import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ui"))
from cc2316_manifest import validate


class ManifestTests(unittest.TestCase):
    def fixture(self, root, version="2.1.316", include_file=True):
        (root / "artifacts" / "frames" / "reference").mkdir(parents=True)
        (root / "artifacts" / "data").mkdir(parents=True)
        (root / "artifacts" / "data" / "baseline.json").write_text(
            json.dumps({"version": "2.1.316", "binary_sha256": "a" * 64}), encoding="utf-8")
        f = root / "artifacts" / "frames" / "reference"
        (f / "index.json").write_text(json.dumps([{"name": "boot", "json": "boot.json", "dimensions": {"cols": 80, "rows": 24}}]), encoding="utf-8")
        if include_file:
            (f / "boot.json").write_text(json.dumps({"target": {"version": version}, "dimensions": {"cols": 80, "rows": 24}}), encoding="utf-8")

    def test_valid_fixture(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            self.assertEqual(validate(root), [])

    def test_missing_capture_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root, include_file=False)
            self.assertTrue(any("missing" in e.lower() for e in validate(root)))

    def test_mixed_version_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root, version="2.1.301")
            self.assertTrue(any("version" in e.lower() for e in validate(root)))

    def test_dimension_mismatch_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            capture = root / "artifacts/frames/reference/boot.json"
            data = json.loads(capture.read_text(encoding="utf-8"))
            data["dimensions"]["rows"] = 25
            capture.write_text(json.dumps(data), encoding="utf-8")
            self.assertTrue(any("dimensions" in e.lower() for e in validate(root)))


if __name__ == "__main__":
    unittest.main()
