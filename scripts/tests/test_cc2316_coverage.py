"""Tests for the P04 coverage ledger. Synthetic data only, built in temp directories."""
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "ui"))
import cc2316_coverage as cov  # noqa: E402

OWNERS_FILE = HERE.parent / "ui" / "cc2316_owners.json"


def make_reference(root: Path, statuses=("VERIFIED", "PARTIAL")):
    (root / "artifacts/frames/reference").mkdir(parents=True)
    (root / "artifacts/data").mkdir(parents=True)
    feats = [{"id": f"F-{i}", "feature": "x", "status": s, "priority": "P1", "test": f"T-{i}"}
             for i, s in enumerate(statuses, 1)]
    (root / "EVIDENCE_MATRIX.json").write_text(json.dumps({"features": feats}), encoding="utf-8")
    (root / "artifacts/frames/reference/index.json").write_text(
        json.dumps([{"name": "alpha__01"}, {"name": "beta__01"}]), encoding="utf-8")
    (root / "artifacts/data/test_results.json").write_text(json.dumps({"results": [
        {"id": "T-1", "test": "alpha", "snapshot": "01", "status": "PASS"}]}), encoding="utf-8")
    (root / "COMMANDS.md").write_text(
        "# c\n\n| Command | Type | Description | Aliases |\n|---|---|---|---|\n"
        "| `/one` | local | d | — |\n| `/one` | local | d | — |\n| `/two` | local | d | — |\n", encoding="utf-8")
    (root / "KEYBINDINGS.md").write_text(
        "# k\n\n## Global\n\n| Key | Action | Status |\n|---|---|---|\n"
        "| `a` | `x:y` | VERIFIED |\n| `b` | `x:z` | BLOCKED |\n\n## Chat\n\n| Key | Action | Status |\n"
        "|---|---|---|\n| `a` | `x:y` | EXPECTED_FAIL |\n", encoding="utf-8")


def owners(**extra):
    base = {"defaults": {"commands": {"owner_task": "P09b", "source_owners": ["k.rs"]},
                         "keybindings": {"owner_task": "P09b", "source_owners": ["k.rs"]}},
            "features": {"F-1": {"owner_task": "P05", "source_owners": ["a.rs"], "tier": "exact"},
                         "F-2": {"owner_task": "P06", "source_owners": ["b.rs"], "tier": "structure"}},
            "goldens": {"alpha__01": {"owner_task": "P05", "source_owners": ["a.rs"], "tier": "exact"},
                        "beta__01": {"owner_task": "NONE", "source_owners": [], "tier": "exception",
                                     "dropped": True, "reason": "D3: dropped"}}}
    base.update(extra)
    return base


class LedgerTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name) / "ref"
        make_reference(self.root)

    def build(self, own=None, results=None, **kw):
        return cov.build_coverage(self.root, self.root / "artifacts/data/test_results.json",
                                  own if own is not None else owners(), results, **kw)

    def row(self, c, rid):
        return next(r for r in c.rows if r["id"] == rid)

    def test_one_row_per_item_and_starting_states(self):
        c = self.build()
        self.assertEqual(c.counts["kinds"], {"feature": 2, "golden": 2, "command": 3, "keybinding": 3})
        self.assertEqual(c.errors, [])
        self.assertEqual({r["state"] for r in c.rows if r["kind"] in ("feature", "golden")}, {"NOT_STARTED"})
        self.assertEqual({r["state"] for r in c.rows if r["kind"] in ("command", "keybinding")},
                         {"UNCLASSIFIED"})

    def test_duplicate_commands_and_cross_context_keys_get_distinct_rows(self):
        c = self.build()
        ids = [r["id"] for r in c.rows]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual(len([r for r in c.rows if r["kind"] == "command"]), 3)
        self.assertEqual(len([r for r in c.rows if r["kind"] == "keybinding"]), 3)

    def test_row_fields_come_from_owners_and_reference(self):
        c = self.build()
        f = self.row(c, "F-2")
        self.assertEqual((f["reference_status"], f["owner_task"], f["tier"]), ("PARTIAL", "P06", "structure"))
        self.assertEqual(f["source_owner"], ["b.rs"])
        self.assertEqual(self.row(c, "F-1")["golden"], ["alpha__01"])
        self.assertEqual(self.row(c, "alpha__01")["reference_status"], "PASS")
        self.assertEqual(self.row(c, "beta__01")["reference_status"], "FRAME_ONLY")
        d = self.row(c, "beta__01")
        self.assertEqual((d["tier"], d["limitation"]), ("exception", "D3: dropped"))

    def test_unassigned_when_no_owner_entry(self):
        own = owners()
        del own["features"]["F-2"]
        c = self.build(own)
        self.assertEqual(self.row(c, "F-2")["owner_task"], "UNASSIGNED")
        self.assertEqual(c.unassigned, ["F-2"])
        self.assertEqual(c.errors, [])

    def test_owner_entry_missing_from_reference_fails(self):
        own = owners()
        own["features"]["F-9"] = {"owner_task": "P05", "source_owners": [], "tier": "exact"}
        own["goldens"]["nope__01"] = {"owner_task": "P05", "source_owners": [], "tier": "exact"}
        c = self.build(own)
        self.assertTrue(any("F-9" in e for e in c.errors))
        self.assertTrue(any("nope__01" in e for e in c.errors))
        self.assertFalse(c.ok)

    def test_bad_tier_fails(self):
        own = owners()
        own["features"]["F-1"]["tier"] = "approx"
        self.assertTrue(any("tier" in e for e in self.build(own).errors))

    def test_exception_without_d1_or_d3_reason_fails(self):
        own = owners()
        own["features"]["F-1"].update(tier="exception", reason="we felt like it")
        self.assertTrue(any("D1 or D3" in e for e in self.build(own).errors))
        own["features"]["F-1"].pop("reason")
        self.assertTrue(any("D1 or D3" in e for e in self.build(own).errors))

    def test_verified_needs_a_passing_result_with_evidence(self):
        for result in ({}, {"result": "PASS"}, {"evidence": "e.txt"}, {"result": "FAIL", "evidence": "e.txt"}):
            c = self.build(results={"F-1": dict(result, state="VERIFIED")})
            self.assertTrue(any("F-1" in e and "VERIFIED" in e for e in c.errors), result)
        c = self.build(results={"F-1": {"state": "VERIFIED", "result": "PASS", "evidence": "e.txt"}})
        self.assertEqual(c.errors, [])
        self.assertEqual(self.row(c, "F-1")["state"], "VERIFIED")
        self.assertEqual(self.row(c, "F-1")["evidence"], "e.txt")

    def test_verified_against_weak_reference_needs_a_limitation(self):
        good = {"state": "VERIFIED", "result": "PASS", "evidence": "e.txt"}
        c = self.build(results={"F-2": good})            # F-2 is PARTIAL in the reference
        self.assertTrue(any("F-2" in e and "limitation" in e for e in c.errors))
        c = self.build(results={"F-2": dict(good, limitation="only the bar is covered")})
        self.assertEqual(c.errors, [])
        self.assertEqual(self.row(c, "F-2")["limitation"], "only the bar is covered")
        # a weak keybinding row (BLOCKED / EXPECTED_FAIL) follows the same rule
        key = next(r["id"] for r in self.build().rows
                   if r["kind"] == "keybinding" and r["reference_status"] == "BLOCKED")
        self.assertTrue(self.build(results={key: dict(good)}).errors)

    def test_weak_reference_rows_carry_a_default_limitation(self):
        c = self.build()
        self.assertIn("PARTIAL", self.row(c, "F-2")["limitation"])
        self.assertEqual(self.row(c, "F-1")["limitation"], "")

    def test_result_for_unknown_id_or_state_fails(self):
        self.assertTrue(any("ghost" in e for e in self.build(results={"ghost": {"state": "FAILED"}}).errors))
        self.assertTrue(any("state" in e for e in self.build(results={"F-1": {"state": "PASSING"}}).errors))

    def test_nothing_is_marked_verified_by_itself(self):
        c = self.build()
        self.assertNotIn("VERIFIED", {r["state"] for r in c.rows})
        self.assertEqual(c.counts["states"].get("VERIFIED", 0), 0)

    def test_results_and_owners_can_be_files(self):
        o, r = Path(self._tmp.name) / "o.json", Path(self._tmp.name) / "r.json"
        o.write_text(json.dumps(owners()), encoding="utf-8")
        r.write_text(json.dumps({"F-1": {"state": "FAILED", "result": "FAIL", "evidence": "e.txt"}}),
                     encoding="utf-8")
        c = self.build(o, r)
        self.assertEqual(self.row(c, "F-1")["state"], "FAILED")

    def test_missing_reference_input_raises(self):
        (self.root / "COMMANDS.md").unlink()
        with self.assertRaises(FileNotFoundError):
            self.build()


class OutputLocationTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name) / "ref"
        make_reference(self.root)

    def test_writes_markdown_and_json_to_out_dir(self):
        out = Path(self._tmp.name) / "evidence"
        cov.build_coverage(self.root, self.root / "artifacts/data/test_results.json", owners(),
                           out_dir=out)
        data = json.loads((out / "coverage.json").read_text(encoding="utf-8"))
        self.assertEqual(len(data["rows"]), 10)
        self.assertIn("| F-1 |", (out / "coverage.md").read_text(encoding="utf-8"))

    def test_refuses_a_path_inside_the_repository(self):
        inside = cov.REPO_ROOT / "target" / "cc2316_cov_should_not_exist"
        with self.assertRaises(ValueError):
            cov.build_coverage(self.root, self.root / "artifacts/data/test_results.json", owners(),
                               out_dir=inside)
        self.assertFalse(inside.exists())

    def test_evidence_dir_reads_environment_with_default(self):
        old = os.environ.pop("CC2316_EVIDENCE", None)
        self.addCleanup(lambda: os.environ.__setitem__("CC2316_EVIDENCE", old) if old else None)
        self.assertEqual(cov.evidence_dir(), Path("C:/Users/sergi/Desktop/davinci-cc2316-evidence"))
        os.environ["CC2316_EVIDENCE"] = str(Path(self._tmp.name) / "x")
        self.assertEqual(cov.evidence_dir(), Path(self._tmp.name) / "x")
        os.environ.pop("CC2316_EVIDENCE")


class OwnersFileTest(unittest.TestCase):
    def test_committed_file_holds_only_ids_and_dav_facts(self):
        data = json.loads(OWNERS_FILE.read_text(encoding="utf-8"))
        self.assertEqual(set(data), {"version", "defaults", "features", "goldens"})
        entries = list(data["features"].values()) + list(data["goldens"].values())
        for e in entries:
            self.assertLessEqual(set(e), {"owner_task", "source_owners", "tier", "reason", "dropped"})
            self.assertIn(e["tier"], cov.TIERS)
            if e["tier"] == "exception":
                self.assertRegex(e["reason"], r"^D[13]: ")
                self.assertLess(len(e["reason"]), 80)
        self.assertEqual(sum(1 for e in data["goldens"].values() if e.get("dropped")), 4)
        self.assertEqual(sum(1 for e in data["goldens"].values()
                             if e["tier"] == "exception" and not e.get("dropped")), 2)


REF = cov.ref_root()


@unittest.skipUnless((REF / "EVIDENCE_MATRIX.json").is_file(), f"no reference package at {REF}")
class RealLedgerTest(unittest.TestCase):
    def test_real_ledger_counts_and_exceptions(self):
        with tempfile.TemporaryDirectory() as out:
            c = cov.build_coverage(REF, REF / "artifacts/data/test_results.json", OWNERS_FILE,
                                   out_dir=Path(out))
            self.assertEqual(c.errors, [])
            self.assertEqual(c.counts["kinds"]["feature"], 77)
            self.assertEqual(c.counts["kinds"]["golden"], 99)
            self.assertEqual(c.counts["kinds"]["command"], 91)
            self.assertEqual(c.counts["kinds"]["keybinding"], 228)
            self.assertEqual(c.counts["states"], {"NOT_STARTED": 176, "UNCLASSIFIED": 319})
            for r in c.rows:
                if r["tier"] == "exception":
                    self.assertRegex(r["limitation"], r"^D[13]: ")
            self.assertTrue((Path(out) / "coverage.md").is_file())


if __name__ == "__main__":
    unittest.main()
