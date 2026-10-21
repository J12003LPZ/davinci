import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("resource_lifecycle", Path(__file__).parents[1] / "resource_lifecycle.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ProcessTreeTests(unittest.TestCase):
    def test_proc_stat_command_names_do_not_shift_fields(self):
        fields = ["S", "12"] + ["0"] * 17 + ["99", "0", "7"]
        parsed = module.parse_stat("41 (name with ) parenthesis) " + " ".join(fields))
        self.assertEqual((parsed["parent"], parsed["started"], parsed["pages"]), (12, 99, 7))

    def test_descendants_count_once_and_zombies_are_not_live_rss(self):
        records = {1: dict(state="S", parent=0, started=10, pages=2),
                   2: dict(state="S", parent=1, started=11, pages=3),
                   3: dict(state="Z", parent=2, started=12, pages=100),
                   4: dict(state="S", parent=0, started=9, pages=100)}
        self.assertEqual(module.tree_rss(records, 1, 10, 4096), 5 * 4096)
        self.assertEqual(module.tree_members(records, 1, 10), [1, 2])

    def test_missing_or_reused_root_is_unknown(self):
        self.assertIsNone(module.tree_rss({}, 1, 10, 4096))
        self.assertIsNone(module.tree_rss({1: dict(state="S", parent=0, started=11, pages=99)}, 1, 10, 4096))


if __name__ == "__main__":
    unittest.main()
