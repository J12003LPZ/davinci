"""Parity claims, pilot reporting and promotion gates are evidence-bound."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from parity_report import (  # noqa: E402
    HarnessArm,
    ParityClaimError,
    PilotReport,
    PromotionDecision,
    ReportMetrics,
    SuiteEvidence,
    performance_parity_claim,
    report_missing_window,
    rollout_gate,
)


def metrics(**overrides: object) -> ReportMetrics:
    values: dict[str, object] = {
        "successful_tasks": 18,
        "total_attempts": 24,
        "logical_requests": 120,
        "fresh_input": 40_000,
        "cache_read": 60_000,
        "output_plus_reasoning": 12_000,
        "wall_ms": 90_000,
        "account_window_delta": None,
    }
    values.update(overrides)
    return ReportMetrics(**values)  # type: ignore[arg-type]


def test_matched_pilot_reports_unknown_values() -> None:
    arms = (
        HarnessArm("davinci_direct", "gpt-6.1-sol", "high", "native-websocket"),
        HarnessArm("davinci_script", "gpt-6.1-sol", "high", "native-websocket"),
    )
    report = PilotReport(arms=arms, metrics=(metrics(), metrics()), failures=("arm-b:timeout",))
    rows = report.as_rows()
    assert [row["arm"] for row in rows] == ["davinci_direct", "davinci_script"]
    # An unobserved allowance window is reported as not measured, never zero.
    assert all(row["accountWindowDelta"] == "not measured" for row in rows)
    assert all(row["safetyFailures"] == 1 for row in rows)
    # Failed attempts stay in the denominator.
    assert rows[0]["successRate"] == pytest.approx(18 / 24)
    assert report_missing_window(metrics()) == "not measured"

    with pytest.raises(ParityClaimError):
        HarnessArm("droid_unknown", "gpt-6.1-sol", "high", "proxy")
    with pytest.raises(ParityClaimError):
        HarnessArm("droid_direct", "gpt-6.1-sol", "high", "")
    with pytest.raises(ParityClaimError):
        ReportMetrics(
            successful_tasks=5,
            total_attempts=2,
            logical_requests=0,
            fresh_input=0,
            cache_read=0,
            output_plus_reasoning=0,
            wall_ms=0,
        )


def test_performance_claim_requires_suite_gate() -> None:
    weak = SuiteEvidence(
        scenarios=17,
        repetitions=1,
        independent_oracle=True,
        success_noninferior=True,
        security_regressions=0,
        transport_confound_bounded=False,
    )
    allowed, reasons = performance_parity_claim(weak)
    assert not allowed
    assert any("17 shared scenarios" in reason for reason in reasons)
    assert any("1 repetitions" in reason for reason in reasons)
    assert any("transport confound" in reason for reason in reasons)

    strong = SuiteEvidence(
        scenarios=150,
        repetitions=3,
        independent_oracle=True,
        success_noninferior=True,
        security_regressions=0,
        transport_confound_bounded=True,
    )
    allowed, reasons = performance_parity_claim(strong)
    assert allowed, reasons

    unsafe = SuiteEvidence(
        scenarios=160,
        repetitions=3,
        independent_oracle=True,
        success_noninferior=True,
        security_regressions=1,
        transport_confound_bounded=True,
    )
    allowed, reasons = performance_parity_claim(unsafe)
    assert not allowed
    assert any("security regression" in reason for reason in reasons)


def test_subscription_savings_not_inferred_from_tokens() -> None:
    # Fewer tokens is not a subscription saving: without an observed allowance
    # window the derived metric is simply unavailable.
    token_only = metrics(fresh_input=1, cache_read=10_000, logical_requests=1)
    assert token_only.verified_success_per_subscription_unit() is None
    assert report_missing_window(token_only) == "not measured"

    observed = metrics(account_window_delta=60)
    assert observed.verified_success_per_subscription_unit() == pytest.approx(18 / 60)
    assert report_missing_window(observed) == "60"


def test_rollout_gate_keeps_unsafe_mode_off() -> None:
    mode, blockers = rollout_gate(
        PromotionDecision(
            behavior_parity=True,
            security_pass=False,
            success_noninferior=True,
            resource_benefit=True,
        )
    )
    assert mode == "off"
    assert any("security" in blocker for blocker in blockers)

    mode, blockers = rollout_gate(
        PromotionDecision(
            behavior_parity=True,
            security_pass=True,
            success_noninferior=True,
            resource_benefit=True,
            unknowns=("no live performance parity",),
        )
    )
    assert mode == "off"
    assert "no live performance parity" in blockers

    mode, blockers = rollout_gate(
        PromotionDecision(
            behavior_parity=True,
            security_pass=True,
            success_noninferior=True,
            resource_benefit=True,
        )
    )
    assert mode == "controlled-opt-in"
    assert blockers == ()


def test_recorded_promotion_decision_matches_the_gate() -> None:
    """The checked-in decision must be the gate's own output, not a claim."""
    import json

    record_path = (
        Path(__file__).resolve().parents[3]
        / "docs"
        / "perf"
        / "droid-gap"
        / "stage1-promotion.json"
    )
    record = json.loads(record_path.read_text(encoding="utf-8"))

    decision = PromotionDecision(
        behavior_parity=record["behaviorParity"],
        security_pass=record["securityPass"],
        success_noninferior=record["successNoninferior"],
        resource_benefit=record["resourceBenefit"],
        unknowns=tuple(record["unknowns"]),
    )
    mode, blockers = rollout_gate(decision)
    assert record["decision"] == mode == "off"
    # Every recorded unknown is a blocker, and the missing evidence is named.
    for unknown in record["unknowns"]:
        assert unknown in blockers, unknown
    assert any("behavior parity" in blocker for blocker in blockers)
    assert any("resource benefit" in blocker for blocker in blockers)

    # The gap record and the Stage 0 addendum both carry the same decision and
    # never claim a re-measurement.
    docs = Path(__file__).resolve().parents[3] / "docs"
    gap = (docs / "superpowers/plans/2026-10-31-droid-gap.md").read_text(
        encoding="utf-8"
    )
    stage0 = (docs / "perf/droid-gap/stage0-report.md").read_text(encoding="utf-8")
    for text in (gap, stage0):
        assert "off" in text
    for text in (gap,):
        assert "rollback" in text.lower()
    assert "does not re-measure" in stage0
    assert "No new performance measurement" in stage0
