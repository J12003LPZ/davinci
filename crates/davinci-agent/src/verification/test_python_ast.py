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
        self.assertEqual(facts["checked_modules"], ["changed", "changed.f"])

    def test_case_table_and_expected_exception(self):
        facts = inspect("from changed import f\ncases = {1: 2, 2: 4}\n"
                        "for value, expected in cases.items():\n assert f(value) == expected\n"
                        "try:\n f(-1)\nexcept ValueError:\n pass\n"
                        "else:\n raise AssertionError('accepted invalid')")
        self.assertEqual(facts["checked_modules"], ["changed", "changed.f"])

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
            "from changed import f\nimport sys\nsys.exit(0)\nassert f(1) == 3",
            "from changed import f\nfrom sys import exit as stop\nstop(0)\nassert f(1) == 3",
            "from changed import f\ndef check():\n import sys\n sys.exit(0)\n assert f(1) == 3\ncheck()",
            "from changed import f\nx = f(1)\nassert (x, 'message')",
            "from changed import f\nx = (f(1), 'message')\nassert x",
            "from changed import f\nassert len((f(1),))",
            "import changed\nassert hasattr(changed, 'f')",
            "from changed import f\nassert callable(f)",
            "from changed import f\nassert f is not None",
            "from changed import f\nassert f == f",
            "from changed import f\nassert f(1) == 3 or True",
            "from changed import f\nt = True\nassert f(1) == 3 or t",
            "from changed import f\nt = True\nx = f(1) == 3 or t\nassert x",
            "from changed import f\nother = f\nassert other",
            "from changed import f\nassert bool(f)",
            "from changed import f\nassert str(f)",
            "from changed import f\nassert isinstance(f, object)",
            "from changed import f\nassert isinstance(f(1), object)",
            "import changed\nf = changed.f\nassert f",
            "import changed\nassert bool(changed.f)",
            "from changed import f\nassert f != None",
            "from changed import f\nassert all([f])",
            "from changed import f\nassert any([f(1), True])",
            "from changed import f\nassert tuple([f(1)])",
            "from changed import f\nfrom builtins import exit as stop\nstop(0)\nassert f(1) == 3",
            "import changed, builtins\nbuiltins.exit(0)\nassert changed.f(1) == 3",
            "import changed, sys\nsys.exit.__call__(0)\nassert changed.f(1) == 3",
            "from builtins import hasattr\nimport changed\nassert hasattr(changed, 'f')",
            "from changed import f\nassert (f(1),) != ()",
            "from changed import f\nx = [f(1)]\nassert x != []",
            "from changed import f\nassert f(1) not in []",
            "from changed import f\nassert (f(1),) != (1, 2)",
            "from changed import f\nx = (f(1),)\ny = (1, 2)\nassert x != y",
            "from changed import f\nassert f in [f]",
            "from changed import f\nassert [f(1)] == [f(1)]",
            "from changed import f\nimport sys\nexit_ = sys.exit\nexit_(0)\nassert f(1) == 3",
        ]
        for source in cases:
            with self.subTest(source=source):
                facts = inspect(source)
                self.assertEqual(facts["checked_modules"], [])
                self.assertEqual(facts["checked_files"], [])

    def test_unittest_overrides_cannot_credit_unexecuted_assertions(self):
        header = "import unittest\nimport changed\n"
        cases = [
            "class T(unittest.TestCase):\n def test_f(self):\n  assert changed.f(1) == 3\n def test_f(self):\n  pass\nunittest.main()",
            "class T(unittest.TestCase):\n def test_f(self):\n  assert changed.f(1) == 3\n def run(self, result):\n  result.startTest(self)\n  result.stopTest(self)\nunittest.main()",
            "class T(unittest.TestCase):\n def test_f(self):\n  self.skipTest('skip')\n  assert changed.f(1) == 3\nunittest.main()",
            "class T(unittest.TestCase):\n def test_f(self):\n  assert changed.f(1) == 3\nT = None\nunittest.main()",
            "class T(unittest.TestCase):\n def test_f(self):\n  assert changed.f(1) == 3\ndef load_tests(loader, tests, pattern):\n return unittest.TestSuite()\nunittest.main()",
            "unittest.main()\nassert changed.f(1) == 3",
            "class T(unittest.TestCase):\n def test_f(self):\n  assert changed.f(1) == 3\ndef T():\n pass\nunittest.main()",
            "class T(unittest.TestCase):\n def test_f(self):\n  self.assertNotIn(changed.f(1), [])\nunittest.main()",
            "def check():\n class T(unittest.TestCase):\n  def test_f(self):\n   assert changed.f(1) == 3\n unittest.main()\ncheck()",
        ]
        for source in cases:
            with self.subTest(source=source):
                self.assertEqual(inspect(header + source)["checked_modules"], [])

    def test_submodule_import_bare_raise_and_deleted_file_check(self):
        facts = inspect("from pkg import lru\nif lru.f(2) != 4:\n raise AssertionError")
        self.assertEqual(facts["checked_modules"], ["pkg", "pkg.lru"])
        facts = inspect("from pathlib import Path\nassert not Path('removed.py').exists()")
        self.assertEqual(facts["checked_files"], ["removed.py"])

    def test_expected_exception_with_bare_assertion_error(self):
        facts = inspect("from changed import f\ntry:\n f(-1)\nexcept ValueError:\n pass\n"
                        "else:\n raise AssertionError")
        self.assertIn("changed", facts["checked_modules"])

    def test_boolean_result_and_conjunction_remain_checks(self):
        for source in ["from changed import f\nx = f(1)\nassert x",
                       "from changed import f\nassert f(1) and f(2)"]:
            self.assertIn("changed", inspect(source)["checked_modules"])

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
