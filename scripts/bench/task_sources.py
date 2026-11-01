"""Versioned, pinned study-task sources and graders.

The Stage 0 research fixtures graded two known-good answers as failures because
the artifact was written to the wrong root, and one task had ambiguous
expectations. This module pins the adapter version, the artifact location and
the grader semantics, and it proves the grader itself with a known-good and a
deliberately bad answer before any scored run uses it.

Nothing here executes a model or spends subscription usage.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from pathlib import Path

# Bump when the adapter or any grader semantics change. A scored run records
# this value, so a historical result can never be silently re-graded.
STUDY_ADAPTER_VERSION = 2

# Research-01/02 wrote their fixture inside the harness directory instead of
# the workspace root; the grader reads only this location.
ARTIFACT_RELPATH = "answer.txt"


class TaskSourceError(ValueError):
    """The adapter or the task is not usable for a scored run."""


@dataclass(frozen=True)
class StudyTask:
    id: str
    prompt: str
    expected: str
    # SHA-256 of the pinned expected artifact text.
    artifact_sha256: str = ""
    adapter_version: int = STUDY_ADAPTER_VERSION


def fixture_digest(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


@dataclass(frozen=True)
class StudyTaskAdapter:
    """Reads the graded artifact from exactly one pinned location."""

    version: int = STUDY_ADAPTER_VERSION
    artifact_relpath: str = ARTIFACT_RELPATH
    notes: tuple[str, ...] = field(default_factory=tuple)

    def artifact_path(self, workspace: Path) -> Path:
        return Path(workspace) / self.artifact_relpath

    def read(self, workspace: Path) -> str:
        path = self.artifact_path(workspace)
        if not path.is_file():
            raise TaskSourceError(
                f"expected artifact at {path}; the research fixtures historically "
                "wrote it to the wrong root"
            )
        return path.read_text(encoding="utf-8")

    def grade(self, task: StudyTask, workspace: Path) -> tuple[bool, str]:
        if task.adapter_version != self.version:
            raise TaskSourceError(
                f"task {task.id} was pinned to adapter v{task.adapter_version}, "
                f"this adapter is v{self.version}"
            )
        try:
            actual = self.read(workspace)
        except TaskSourceError as error:
            return False, str(error)
        normalized = actual.strip()
        if normalized != task.expected.strip():
            return False, f"expected {task.expected.strip()!r}, got {normalized!r}"
        if task.artifact_sha256 and fixture_digest(task.expected) != task.artifact_sha256:
            return False, "pinned fixture hash does not match the expected artifact"
        return True, "matches the pinned expectation"


def known_good_solution() -> str:
    return "7\n8\n"


def deliberately_bad_solution() -> str:
    # Research-03's ambiguous 7/8-digit expectation: only one row, wrong width.
    return "7\n"


def default_tasks() -> tuple[StudyTask, ...]:
    expected = known_good_solution()
    return (
        StudyTask(
            id="research-01",
            prompt="Write the two integers, one per line.",
            expected=expected,
            artifact_sha256=fixture_digest(expected),
        ),
    )
