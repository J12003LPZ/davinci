import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ui"))
from cc2316_compare import compare


class ComparatorTests(unittest.TestCase):
    def fixture(self, fg="#ffffff", x=2, bold=False):
        return {"dimensions": {"cols": 2, "rows": 1},
                "cursor": {"x": x, "y": 0, "hidden": True},
                "modes": {"alternate_screen": True, "mouse_sgr": True},
                "cells": [[{"chars": "A", "fg": fg, "bold": bold}, {"chars": " ", "fg": None}]]}

    def test_identical(self):
        a = self.fixture()
        self.assertEqual(compare(a, a), [])

    def test_color_difference_is_not_normalized(self):
        differences = compare(self.fixture(), self.fixture(fg="#eeeeee"))
        self.assertTrue(any("fg" in d for d in differences))

    def test_cursor_difference(self):
        self.assertTrue(any("cursor" in d for d in compare(self.fixture(), self.fixture(x=1))))

    def test_text_difference(self):
        other = self.fixture()
        other["cells"][0][0]["chars"] = "B"
        self.assertTrue(any("chars" in d for d in compare(self.fixture(), other)))

    def test_mode_difference(self):
        other = self.fixture()
        other["modes"]["mouse_sgr"] = False
        self.assertTrue(any("mouse_sgr" in d for d in compare(self.fixture(), other)))


if __name__ == "__main__":
    unittest.main()
