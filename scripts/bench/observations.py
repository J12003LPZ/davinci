"""Parse host-owned activity, retaining unavailable or incomplete evidence."""


def activity(events):
    logical, ended, attempts, attempts_ended = set(), set(), set(), set()
    reasons, auto_ids, started_tools, ended_batches = {}, set(), set(), set()
    causes = {}
    auto_runs = batch_children = top_level = after_reminder = 0
    telemetry_seen = overflow = False
    unknown_outcome = False
    mutation_generation = None
    mutation_invalid = False
    mutations_after_reminder = 0
    leaf_baseline = leaf_max = None
    leaf_invalid = False
    tools = {}
    for event in events:
        kind = event.get("type")
        if kind == "mutation_observation":
            leaves = event.get("executed_leaf_operations")
            if event.get("schema_version") != 1 or type(leaves) is not int or leaves < 0:
                leaf_invalid = True
            elif leaf_baseline is None:
                leaf_baseline = leaf_max = leaves
            else:
                leaf_max = max(leaf_max, leaves)
            generation = event.get("generation")
            if event.get("schema_version") != 1 or type(generation) is not int or generation < 0:
                mutation_invalid = True
            elif mutation_generation is None:
                mutation_generation = generation
                if reasons:
                    mutation_invalid = True
            else:
                if reasons:
                    mutations_after_reminder += max(0, generation - mutation_generation)
                mutation_generation = max(generation, mutation_generation)
        elif kind == "provider_observation":
            observation = event.get("observation")
            if not isinstance(observation, dict) or observation.get("schema_version") != 1:
                overflow = True
                continue
            request = observation.get("logical_request_id")
            purpose = observation.get("purpose")
            if not isinstance(request, str) or not request or purpose not in ("coding", "prewarm", "jev"):
                overflow = True
                continue
            telemetry_seen = True
            key = (purpose, request)
            phase = observation.get("kind")
            if phase in ("attempt_end", "logical_end") and observation.get("status") == "unknown":
                unknown_outcome = True
            if phase == "logical_start":
                if key not in logical and reasons and purpose == "coding":
                    after_reminder += 1
                logical.add(key)
            elif phase == "logical_end":
                ended.add(key)
            elif phase in ("attempt_start", "attempt_end"):
                attempt = observation.get("attempt_id")
                if type(attempt) is not int or attempt < 1:
                    overflow = True
                    continue
                target = attempts if phase == "attempt_start" else attempts_ended
                target.add((purpose, request, attempt))
            elif phase == "telemetry_overflow":
                overflow = True
            else:
                overflow = True
        elif kind == "message_end":
            message = event.get("message") or {}
            if not isinstance(message, dict):
                continue
            reason = message.get("davinciCapabilityReminder")
            if isinstance(reason, str) and reason:
                reasons[reason] = reasons.get(reason, 0) + 1
                # Why the last check after the change was not credited.
                cause = message.get("davinciCapabilityReminderReason")
                cause = cause if isinstance(cause, str) and cause else "none_recorded"
                causes[cause] = causes.get(cause, 0) + 1
            if message.get("davinciHarnessVerification") is True:
                content = message.get("content")
                for block in content if isinstance(content, list) else []:
                    if isinstance(block, dict) and block.get("type") == "toolCall":
                        if isinstance(block.get("id"), str):
                            auto_ids.add(block["id"])
        elif kind == "tool_execution_start":
            call_id = event.get("toolCallId")
            call_id = call_id if isinstance(call_id, str) else None
            if call_id and call_id in started_tools:
                continue
            if call_id:
                started_tools.add(call_id)
            if call_id in auto_ids:
                auto_runs += 1
            else:
                top_level += 1
                name = event.get("toolName") or "?"
                name = name if isinstance(name, str) else "?"
                tools[name] = tools.get(name, 0) + 1
        elif kind == "tool_execution_end":
            details = event.get("details") or {}
            call_id = event.get("toolCallId")
            call_id = call_id if isinstance(call_id, str) else None
            if isinstance(details, dict) and details.get("batch") is True and call_id not in ended_batches:
                ended_batches.add(call_id)
                operations = details.get("operations")
                batch_children += sum(
                    1 for child in (operations if isinstance(operations, list) else [])
                    if isinstance(child, dict) and child.get("status") in ("ok", "error"))
    available = telemetry_seen and not overflow
    count = lambda values, purpose: sum(key[0] == purpose for key in values) if available else None
    return {
        "logical_requests": count(logical, "coding"),
        "requests": count(logical, "coding"),
        "provider_attempts": len(attempts) if available else None,
        "coding_provider_attempts": count(attempts, "coding"),
        "prewarm_attempts": count(attempts, "prewarm"),
        "jev_attempts": count(attempts, "jev") if any(key[0] == "jev" for key in logical | ended) else None,
        "request_metrics_available": available,
        "request_metrics_complete": available and not unknown_outcome and logical == ended and attempts == attempts_ended
            and all((purpose, request) in logical for purpose, request, _ in attempts),
        "requests_after_first_reminder": after_reminder if available else None,
        "gate_reminders": reasons,
        "gate_reminder_causes": causes,
        "auto_verify_runs": auto_runs,
        "tool_calls": top_level,
        "tools": tools,
        "batch_children_reported": batch_children,
        "executed_leaf_operations": leaf_max - leaf_baseline if leaf_baseline is not None and not leaf_invalid else None,
        "executed_leaf_scope": "leaf tool dispatches including failures; excludes batch wrappers and journal replay",
        "mutations_after_reminder": mutations_after_reminder if mutation_generation is not None and not mutation_invalid else None,
        "mutation_metric_scope": "completion-ledger generations",
    }
