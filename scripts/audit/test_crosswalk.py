import contextlib
import csv
import io
import tempfile
import unittest
from pathlib import Path

import crosswalk as cw


def row(fid, disposition="NOT_APPLICABLE", anchor="", gap="", reason="r"):
    values = dict.fromkeys(cw.HEADER, "")
    values.update(
        finding_id=fid, disposition=disposition, test_anchor=anchor, gap_id=gap, decision_reason=reason
    )
    return values


class CrosswalkTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.crates = self.dir / "crates"
        (self.crates / "a" / "src").mkdir(parents=True)
        (self.crates / "a" / "src" / "lib.rs").write_text(
            "#[test]\nfn real_test() {}\n", encoding="utf-8"
        )
        self.index = cw.FnIndex(self.dir, self.crates)
        self.register = ["F-01", "F-02", "F-03"]

    def check(self, rows, register=None):
        return cw.validate(rows, cw.HEADER, register or self.register, self.index)

    def good_rows(self):
        return [row(fid) for fid in self.register]

    def test_clean_file_passes(self):
        self.assertEqual(self.check(self.good_rows()), [])

    def test_duplicate_id_fails(self):
        rows = self.good_rows() + [row("F-02")]
        problems = self.check(rows)
        self.assertTrue(any("duplicate finding id F-02" in p for p in problems), problems)

    def test_missing_id_fails(self):
        problems = self.check(self.good_rows()[:2])
        self.assertTrue(any("missing finding id F-03" in p for p in problems), problems)

    def test_extra_id_fails(self):
        problems = self.check(self.good_rows() + [row("F-99")])
        self.assertTrue(any("extra finding id F-99" in p for p in problems), problems)

    def test_order_must_match_the_register(self):
        rows = self.good_rows()
        rows[0], rows[1] = rows[1], rows[0]
        self.assertTrue(any("order differs" in p for p in self.check(rows)))

    def test_bad_disposition_fails(self):
        rows = self.good_rows()
        rows[1]["disposition"] = "DONE"
        problems = self.check(rows)
        self.assertTrue(any("bad disposition 'DONE'" in p for p in problems), problems)

    def test_unknown_test_anchor_fails(self):
        rows = self.good_rows()
        rows[0].update(disposition=cw.IMPLEMENTED, test_anchor="crates/a/src/lib.rs::no_such_test")
        problems = self.check(rows)
        self.assertTrue(any("does not resolve" in p for p in problems), problems)

    def test_empty_anchor_on_implemented_fails(self):
        rows = self.good_rows()
        rows[0]["disposition"] = cw.IMPLEMENTED
        self.assertTrue(any("empty test anchor" in p for p in self.check(rows)))

    def test_known_anchor_forms_resolve(self):
        for anchor in (
            "crates/a/src/lib.rs::real_test",
            "crates/a/src/lib.rs::tests::real_test",
            "real_test",
        ):
            self.assertTrue(self.index.resolves(anchor), anchor)

    def test_anchor_must_be_in_the_named_file_and_planned_tests_do_not_count(self):
        (self.crates / "a" / "src" / "other.rs").write_text("fn other() {}\n", encoding="utf-8")
        self.assertFalse(self.index.resolves("crates/a/src/other.rs::real_test"))
        self.assertFalse(self.index.resolves("crates/a/src/missing.rs::real_test"))
        self.assertFalse(self.index.resolves("WRITE:crates/a/src/lib.rs::real_test"))

    def test_fix_anchors_downgrades_only_unresolved_rows(self):
        rows = [
            row("F-01", cw.IMPLEMENTED, "crates/a/src/lib.rs::real_test"),
            row("F-02", cw.IMPLEMENTED, "crates/a/src/lib.rs::gone"),
            row("F-03", cw.IMPLEMENTED, ""),
        ]
        self.assertEqual(cw.fix_anchors(rows, self.index), ["F-02", "F-03"])
        self.assertEqual(rows[0]["disposition"], cw.IMPLEMENTED)
        self.assertEqual(rows[1]["disposition"], cw.UNVERIFIED)
        self.assertIn("anchor not found: crates/a/src/lib.rs::gone", rows[1]["decision_reason"])
        self.assertEqual(self.check(rows), [])
        self.assertEqual(cw.fix_anchors(rows, self.index), [])

    def test_closures_need_an_open_state_a_matching_gap_and_a_real_anchor(self):
        anchor = "crates/a/src/lib.rs::real_test"
        rows = [
            row("F-01", "WEAK", gap="G-12"),
            row("F-02", "BLOCKED", gap="G-12"),
            row("F-03", "MISSING", gap="G-99"),
        ]
        mapping = [
            ("F-01", "G-12", anchor, "why"),
            ("F-02", "G-12", anchor, "why"),
            ("F-03", "G-12", anchor, "why"),
        ]
        closed, notes = cw.apply_closures(rows, self.index, mapping)
        self.assertEqual(closed, ["F-01"])
        self.assertEqual(len(notes), 2)
        self.assertEqual(rows[0]["disposition"], cw.IMPLEMENTED)
        self.assertEqual((rows[1]["disposition"], rows[2]["disposition"]), ("BLOCKED", "MISSING"))
        bad = [row("F-01", "WEAK", gap="G-12")]
        closed, notes = cw.apply_closures(bad, self.index, [("F-01", "G-12", "crates/a/src/lib.rs::gone", "x")])
        self.assertEqual((closed, bad[0]["disposition"]), ([], "WEAK"))

    def test_merge_orders_by_the_register_and_cli_exit_codes(self):
        register = self.dir / "register.csv"
        register.write_text("﻿Finding ID,Subsystem\nF-01,a\nF-02,b\nF-03,c\n", encoding="utf-8")
        parts = self.dir / "parts"
        parts.mkdir()
        for name, ids in (("part1.csv", ["F-03", "F-01"]), ("part2.csv", ["F-02"])):
            with open(parts / name, "w", encoding="utf-8", newline="") as handle:
                writer = csv.DictWriter(handle, fieldnames=cw.HEADER, lineterminator="\n")
                writer.writeheader()
                writer.writerows(row(fid) for fid in ids)
        out = self.dir / "merged.csv"
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(
                cw.main(["merge", "--parts", str(parts), "--out", str(out), "--register", str(register)]), 0
            )
            args = ["check", "--csv", str(out), "--register", str(register), "--crates", str(self.crates)]
            self.assertEqual(cw.main(args), 0)
        _, merged = cw.read_rows(out)
        self.assertEqual([r["finding_id"] for r in merged], ["F-01", "F-02", "F-03"])
        merged[1].update(disposition=cw.IMPLEMENTED, test_anchor="no_such_test")
        cw.write_rows(out, merged)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(cw.main(args), 1)
            self.assertEqual(cw.main(args + ["--fix-anchors"]), 0)


if __name__ == "__main__":
    unittest.main()
