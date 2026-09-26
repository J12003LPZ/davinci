"""Offline acceptance and adversarial cases for syntax-only inspection."""
import importlib.util
import pathlib
import json
import unittest


def inspect(source):
    path = pathlib.Path(__file__).with_name("python_ast.py")
    spec = importlib.util.spec_from_file_location("verification_ast", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.inspect_source(source)


class InlineSyntaxTests(unittest.TestCase):
    def test_concrete_check_and_value_propagation(self):
        facts = inspect("from changed import f\nx = f(2)\nassert x == 4")
        self.assertEqual(facts["checked_modules"], ["changed"])

    def test_case_table_and_expected_exception(self):
        facts = inspect("from changed import f\ncases = {1: 2, 2: 4}\n"
                        "for value, expected in cases.items():\n assert f(value) == expected\n"
                        "try:\n f(-1)\nexcept ValueError:\n pass\n"
                        "else:\n raise AssertionError('accepted invalid')")
        self.assertEqual(facts["checked_modules"], ["changed"])

    def test_literal_file_load(self):
        facts = inspect("from pathlib import Path\nx = Path('data.json').read_text()\nassert x == 'ok'")
        self.assertEqual(facts["checked_files"], ["data.json"])

    def test_noops_and_masked_assertions_are_unknown(self):
        cases = [
            "from changed import f\ndef check(f):\n return f(1)\nassert check(lambda x: 1) == 1",
            "from changed import f\ndef check():\n return f(1)\nf = lambda x: 1\nassert check() == 1",
            "from changed import f\nprint(f(1))", "import changed\npass",
            "import changed\nassert True", "import changed\nassert 1 == 1",
            "import changed\nassert 'changed.py'", "import changed\nif False:\n assert changed.f(1) == 2",
            "from changed import f\ntry:\n assert f(1) == 2\nexcept AssertionError:\n pass",
            "from changed import f\ntry:\n assert f(1) == 2\nexcept:\n pass",
            "from changed import f\nfor x in []:\n assert f(x) == 2",
            "import changed\nexec('assert changed.f(1) == 2')",
            "import changed\nassert True or changed.f(1) == 2",
            "import changed\nchanged = 1\nassert changed == 1",
            "import changed\ndef check():\n assert changed.f(1) == 2",
            "import changed\nassert changed", "from pathlib import Path\nassert Path('changed.py')",
            "from changed import f\ndef check():\n return f(1)\ncheck = lambda: True\nassert check()",
        ]
        for source in cases:
            with self.subTest(source=source):
                facts = inspect(source)
                self.assertEqual(facts["checked_modules"], [])
                self.assertEqual(facts["checked_files"], [])

    def test_bounds_and_invalid_syntax(self):
        for source in ("x" * 65537, "assert (", ""):
            self.assertEqual(inspect(source)["checked_modules"], [])

    def test_recorded_behavioral_checks(self):
        cases = json.loads(pathlib.Path(__file__).with_name("recorded_checks.json").read_text())
        for case in cases:
            source = case["command"].split("<<'PY'\n", 1)[1].rsplit("\nPY", 1)[0]
            if "assert" not in source and "unittest.main()" not in source:
                continue  # Diagnostic print-only runs intentionally remain unknown.
            with self.subTest(recording=case["recording"], source=source):
                facts = inspect(source)
                self.assertTrue(facts["checked_modules"] or facts["checked_files"])


if __name__ == "__main__":
    unittest.main()
