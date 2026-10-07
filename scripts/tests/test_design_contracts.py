"""The design contract generator reads Rust DTOs with a small grammar.

Run: python -m unittest discover -s scripts/tests -p "test_design_contracts.py"
"""
import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "design-contracts.py"
spec = importlib.util.spec_from_file_location("design_contracts", SCRIPT)
contracts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contracts)


class StripComments(unittest.TestCase):
    def test_a_documented_field_keeps_one_field_per_comma(self):
        source = '''pub struct Source {
    /// The host fills this in from the file's bytes; a model may leave it out.
    #[serde(default)]
    pub source_hash: String,
    // A comment with commas, <brackets> and {braces}, still one field.
    pub path: String,
}'''
        fields = contracts.split(contracts.strip_comments(source).split("{", 1)[1].rsplit("}", 1)[0])
        self.assertEqual(fields, ["#[serde(default)]\n    pub source_hash: String", "pub path: String"])

    def test_double_slash_inside_code_is_kept(self):
        line = '    #[serde(rename = "https://example.com")] pub url: String,'
        self.assertEqual(contracts.strip_comments(line), line)

    def test_the_shipped_declarations_are_current(self):
        output = contracts.OUTPUT.read_text()
        self.assertEqual(contracts.generate(), output)


if __name__ == "__main__":
    unittest.main()
