#!/usr/bin/env python3
"""P19: run the DaVinci parity suite by test ID and write a JSON report.

The adapter runs the same `test_cc2316_*.py` IDs the coverage ledger names,
against DaVinci snapshots produced by `cc2316_replay`. It is the runner behind
`cc2316_report.py --run`: the report reads this JSON instead of scraping text.

Usage:
    python scripts/ui/run_suite.py [--ids ID [ID ...]] [--out PATH]
        [--pattern test_cc2316_*.py] [--start-dir scripts/tests]

Exit code is 0 only when every selected test passed (skips and expected
failures do not fail the run, as in unittest).
"""
from __future__ import annotations

import argparse
import json
import sys
import time
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


class RecordingResult(unittest.TextTestResult):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.records = {}
        self._started = {}

    def _begin(self, test):
        self._started[test.id()] = time.perf_counter()

    def _finish(self, test, outcome, detail=""):
        started = self._started.pop(test.id(), time.perf_counter())
        self.records[test.id()] = {
            "id": test.id(),
            "outcome": outcome,
            "seconds": round(time.perf_counter() - started, 3),
            "detail": detail,
        }

    def startTest(self, test):
        self._begin(test)
        super().startTest(test)

    def addSuccess(self, test):
        super().addSuccess(test)
        self._finish(test, "passed")

    def addFailure(self, test, err):
        super().addFailure(test, err)
        self._finish(test, "failed", self._exc_info_to_string(err, test))

    def addError(self, test, err):
        super().addError(test, err)
        self._finish(test, "error", self._exc_info_to_string(err, test))

    def addSkip(self, test, reason):
        super().addSkip(test, reason)
        self._finish(test, "skipped", reason)

    def addExpectedFailure(self, test, err):
        super().addExpectedFailure(test, err)
        self._finish(test, "expected_failure", self._exc_info_to_string(err, test))

    def addUnexpectedSuccess(self, test):
        super().addUnexpectedSuccess(test)
        self._finish(test, "unexpected_success")


def selected(loader, pattern, start_dir, ids):
    suite = loader.discover(str(start_dir), pattern=pattern, top_level_dir=str(start_dir))
    chosen = unittest.TestSuite()

    def walk(item):
        if isinstance(item, unittest.TestSuite):
            for child in item:
                walk(child)
            return
        if not ids or item.id() in ids:
            chosen.addTest(item)

    walk(suite)
    return chosen


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ids", nargs="*", default=[])
    parser.add_argument("--out", type=Path,
                        default=Path("C:/Users/sergi/Desktop/davinci-cc2316-evidence/suite.json"))
    parser.add_argument("--pattern", default="test_cc2316_*.py")
    parser.add_argument("--start-dir", type=Path, default=REPO / "scripts" / "tests")
    args = parser.parse_args(argv)

    loader = unittest.TestLoader()
    suite = selected(loader, args.pattern, args.start_dir, set(args.ids))
    runner = unittest.TextTestRunner(verbosity=2, resultclass=RecordingResult)
    result = runner.run(suite)

    outcome = {
        "tests": len(result.records),
        "failures": len(result.failures),
        "errors": len(result.errors),
        "skipped": len(result.skipped),
        "expected_failures": len(result.expectedFailures),
        "unexpected_successes": len(result.unexpectedSuccesses),
        "seconds": round(sum(r["seconds"] for r in result.records.values()), 2),
        "records": sorted(result.records.values(), key=lambda r: r["id"]),
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(outcome, indent=1), encoding="utf-8")
    print(json.dumps({k: outcome[k] for k in
                      ("tests", "failures", "errors", "skipped",
                       "expected_failures", "unexpected_successes")}))
    print("wrote", args.out)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
