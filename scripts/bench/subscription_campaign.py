"""Subscription-only campaign policy; actual-send accounting lives in DaVinci."""
import json
from pathlib import Path
import time
import uuid

SPEND_POLICY = ("No API spend authorized. Subscription-only campaign. Budget by subscription "
                "usage allowance, request count, task count, and wall-clock time; "
                "API-equivalent dollars are reporting-only.")
FIELDS = {"schema_version", "billing", "model", "effort", "max_requests", "max_tasks", "max_wall_seconds"}


def prepare(policy, root, launches, harnesses, model, effort, settings, *, now=None):
    if not isinstance(policy, dict) or set(policy) != FIELDS:
        raise ValueError("subscription policy requires exact schema fields; no USD spend authorization")
    if (type(policy["schema_version"]) is not int or policy["schema_version"] != 1
            or policy["billing"] != "subscription-only"
            or policy["model"] != model or model != "gpt-6-luna"
            or policy["effort"] != effort or effort != "high" or harnesses != ["davinci"]):
        raise ValueError("subscription baseline must be DaVinci gpt-6-luna/high via Codex OAuth")
    for field in ("max_requests", "max_tasks", "max_wall_seconds"):
        if type(policy[field]) is not int or not 0 < policy[field] <= 2**32:
            raise ValueError("subscription policy needs positive finite " + field)
    if type(launches) is not int or not 0 < launches <= policy["max_tasks"]:
        raise ValueError("campaign exceeds the subscription task cap")
    retry = settings.get("retry", {})
    if (settings.get("transport") != "sse" or settings.get("effortPolicy") != "fixed"
            or retry.get("enabled") is not False or retry.get("provider", {}).get("maxRetries") != 0):
        raise ValueError("subscription campaign requires fixed effort, SSE and disabled retries")
    now = time.time() if now is None else now
    root = Path(root).resolve()
    budget = {"root_id": "subscription-" + uuid.uuid4().hex,
              "ledger": str(root / "subscription-ledger.json"),
              "limits": {"max_requests": policy["max_requests"], "max_output_tokens": None,
                         "max_cost_microusd": None,
                         "deadline_unix_ms": int((now + policy["max_wall_seconds"]) * 1000),
                         "codex_subscription": {"model": model, "effort": effort}},
              "per_attempt_max_output_tokens": None}
    return {"policy": dict(policy), "spend_policy": SPEND_POLICY,
            "allowance_enforcement": "unavailable; hard request/task/time caps",
            "root_budget": budget, "config_path": str(root / "subscription-budget.json")}


def remaining_seconds(config, *, now=None):
    remaining = config["root_budget"]["limits"]["deadline_unix_ms"] / 1000 - (
        time.time() if now is None else now)
    if remaining <= 0:
        raise ValueError("subscription campaign wall-clock allowance exhausted")
    return remaining


def _read_ledger(config):
    ledger = json.loads(Path(config["root_budget"]["ledger"]).read_text(encoding="utf-8"))
    if (ledger["schema_version"] != 1 or ledger["root"] != config["root_budget"]["root_id"]
            or ledger["limits"] != config["root_budget"]["limits"]
            or not isinstance(ledger["reservations"], dict)
            or len(ledger["reservations"]) > config["policy"]["max_requests"]):
        raise ValueError("subscription ledger identity or limits changed")
    return ledger


def snapshot(config):
    try:
        ledger = _read_ledger(config)
    except FileNotFoundError:
        ledger = {"halted": False, "reservations": {}}
    reservations = ledger["reservations"]
    if (ledger["halted"] is not False
            or any(r.get("disposition") != "committed" for r in reservations.values())
            or reservations != config.get("previous_reservations", {})):
        raise ValueError("subscription ledger changed or contains unresolved requests")
    return reservations


def stop_reason(config, exit_code, before=None):
    if exit_code != 0:
        return "subscription_process_failed"
    try:
        ledger = _read_ledger(config)
        reservations = ledger["reservations"]
        if ledger["halted"] is not False:
            return "subscription_ledger_halted"
        if not reservations or any(r.get("disposition") != "committed" for r in reservations.values()):
            return "subscription_unresolved_request"
        if before is not None:
            if any(reservations.get(key) != value for key, value in before.items()):
                return "subscription_ledger_replaced"
            if not (reservations.keys() - before.keys()):
                return "subscription_no_new_request"
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        return "subscription_missing_ledger"
    config["previous_reservations"] = reservations
    return None
