"""Matched-parity reporting and promotion gates.

These are the offline guards around the live arms. They exist so a pilot can
never be reported as parity, and a token improvement can never be reported as
subscription savings, without the evidence the plans require.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional

# The official competitor gate: shared scenarios and repetitions per harness.
SUITE_MIN_SCENARIOS = 150
SUITE_MIN_REPETITIONS = 3

ARM_NAMES = (
    "davinci_direct",
    "davinci_script",
    "droid_direct",
    "droid_script",
)


class ParityClaimError(ValueError):
    """The requested claim is not supported by the recorded evidence."""


@dataclass(frozen=True)
class HarnessArm:
    name: str
    model: str
    effort: str
    route: str

    def __post_init__(self) -> None:
        if self.name not in ARM_NAMES:
            raise ParityClaimError(f"unknown harness arm: {self.name}")
        if not self.route:
            raise ParityClaimError("every arm records its route; the proxy confound is retained")


@dataclass(frozen=True)
class ReportMetrics:
    successful_tasks: int
    total_attempts: int
    logical_requests: int
    fresh_input: int
    cache_read: int
    output_plus_reasoning: int
    wall_ms: int
    # Only present when the provider's allowance window was actually observed.
    account_window_delta: Optional[int] = None

    def __post_init__(self) -> None:
        if self.total_attempts < self.successful_tasks:
            raise ParityClaimError("attempts include every failed and unfinished run")

    def success_rate(self) -> float:
        if self.total_attempts == 0:
            return 0.0
        return self.successful_tasks / self.total_attempts

    def verified_success_per_subscription_unit(self) -> Optional[float]:
        """None when no allowance window was observed. Tokens are not a
        substitute: cached tokens still count toward rate limits."""
        if not self.account_window_delta:
            return None
        return self.successful_tasks / self.account_window_delta


def report_missing_window(metrics: ReportMetrics) -> str:
    if metrics.account_window_delta is None:
        return "not measured"
    return str(metrics.account_window_delta)


@dataclass(frozen=True)
class PilotReport:
    arms: tuple[HarnessArm, ...]
    metrics: tuple[ReportMetrics, ...]
    failures: tuple[str, ...] = field(default_factory=tuple)

    def __post_init__(self) -> None:
        if len(self.arms) != len(self.metrics):
            raise ParityClaimError("every arm reports its own metrics, including failures")

    def as_rows(self) -> list[dict[str, object]]:
        rows = []
        for arm, metric in zip(self.arms, self.metrics):
            rows.append(
                {
                    "arm": arm.name,
                    "model": arm.model,
                    "effort": arm.effort,
                    "route": arm.route,
                    "successRate": metric.success_rate(),
                    "logicalRequests": metric.logical_requests,
                    "freshInput": metric.fresh_input,
                    "cacheRead": metric.cache_read,
                    "outputPlusReasoning": metric.output_plus_reasoning,
                    "wallMs": metric.wall_ms,
                    "accountWindowDelta": report_missing_window(metric),
                    "safetyFailures": len(self.failures),
                }
            )
        return rows


@dataclass(frozen=True)
class SuiteEvidence:
    scenarios: int
    repetitions: int
    independent_oracle: bool
    success_noninferior: bool
    security_regressions: int
    transport_confound_bounded: bool


def performance_parity_claim(evidence: SuiteEvidence) -> tuple[bool, tuple[str, ...]]:
    """Parity is claimed only through the official suite gate."""
    failures: list[str] = []
    if evidence.scenarios < SUITE_MIN_SCENARIOS:
        failures.append(
            f"{evidence.scenarios} shared scenarios < {SUITE_MIN_SCENARIOS}"
        )
    if evidence.repetitions < SUITE_MIN_REPETITIONS:
        failures.append(f"{evidence.repetitions} repetitions < {SUITE_MIN_REPETITIONS}")
    if not evidence.independent_oracle:
        failures.append("no independent grading oracle")
    if not evidence.success_noninferior:
        failures.append("task success is not noninferior")
    if evidence.security_regressions:
        failures.append(f"{evidence.security_regressions} security regression(s)")
    if not evidence.transport_confound_bounded:
        failures.append("transport confound (native WebSocket vs proxy) is unbounded")
    return (not failures, tuple(failures))


@dataclass(frozen=True)
class PromotionDecision:
    behavior_parity: bool
    security_pass: bool
    success_noninferior: bool
    resource_benefit: bool
    unknowns: tuple[str, ...] = ()


def rollout_gate(decision: PromotionDecision) -> tuple[str, tuple[str, ...]]:
    """Return the mode to ship and the reasons it is not promoted."""
    blockers: list[str] = []
    if not decision.behavior_parity:
        blockers.append("behavior parity not established")
    if not decision.security_pass:
        blockers.append("security gate not passed")
    if not decision.success_noninferior:
        blockers.append("task success is not noninferior")
    if not decision.resource_benefit:
        blockers.append("no measured resource benefit")
    blockers.extend(decision.unknowns)
    if blockers:
        return "off", tuple(blockers)
    return "controlled-opt-in", ()
