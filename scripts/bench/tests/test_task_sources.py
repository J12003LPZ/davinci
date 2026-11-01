"""The research adapter must grade a known-good answer as a pass and a
deliberately bad answer as a failure, from the pinned artifact location."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from task_sources import (  # noqa: E402
    ARTIFACT_RELPATH,
    STUDY_ADAPTER_VERSION,
    StudyTaskAdapter,
    TaskSourceError,
    deliberately_bad_solution,
    fixture_digest,
    known_good_solution,
)

from task_sources import StudyTask, default_tasks  # noqa: E402


def test_research_fixtures_grade_correctly(tmp_path: Path) -> None:
    adapter = StudyTaskAdapter()
    task = default_tasks()[0]

    good = tmp_path / "good"
    good.mkdir()
    (good / ARTIFACT_RELPATH).write_text(known_good_solution(), encoding="utf-8")
    passed, reason = adapter.grade(task, good)
    assert passed, reason

    bad = tmp_path / "bad"
    bad.mkdir()
    (bad / ARTIFACT_RELPATH).write_text(
        deliberately_bad_solution(), encoding="utf-8"
    )
    passed, reason = adapter.grade(task, bad)
    assert not passed, reason

    # The historical false negative: the artifact landed in the harness dir
    # instead of the workspace root.
    wrong_root = tmp_path / "wrong-root"
    wrong_root.mkdir()
    (wrong_root / "harness").mkdir()
    (wrong_root / "harness" / ARTIFACT_RELPATH).write_text(
        known_good_solution(), encoding="utf-8"
    )
    passed, reason = adapter.grade(task, wrong_root)
    assert not passed
    assert "expected artifact" in reason

    # A fixture pinned to another adapter version is never silently re-graded.
    legacy = StudyTask(
        id="research-legacy",
        prompt="legacy",
        expected=known_good_solution(),
        adapter_version=STUDY_ADAPTER_VERSION - 1,
    )
    with pytest.raises(TaskSourceError):
        adapter.grade(legacy, good)

    # A mismatched pinned hash is a failure, not a pass.
    tampered = StudyTask(
        id="research-tampered",
        prompt="tampered",
        expected=known_good_solution(),
        artifact_sha256=fixture_digest("something else"),
    )
    passed, reason = adapter.grade(tampered, good)
    assert not passed
    assert "hash" in reason
