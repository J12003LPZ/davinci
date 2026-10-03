// Token observations, never an estimate of included subscription allowance.
const count = (value) => Number.isSafeInteger(value) && value >= 0 ? value : null;

export function codexUsage(usage) {
  const total = count(usage?.input_tokens);
  const cached = count(usage?.cached_input_tokens);
  return {
    input: total !== null && cached !== null && cached <= total ? total - cached : null,
    cached: total !== null && cached !== null && cached <= total ? cached : null,
    // Codex input_tokens includes cached input; no separate write bucket.
    cacheWrite: 0,
    output: count(usage?.output_tokens),
  };
}

export function davinciUsage(usage) {
  return {
    input: count(usage?.input),
    cached: count(usage?.cacheRead),
    cacheWrite: count(usage?.cacheWrite),
    output: count(usage?.output),
  };
}

export function summarizeUsage(records) {
  const fields = ["input", "cached", "cacheWrite", "output"];
  const totals = Object.fromEntries(fields.map((field) => {
    const values = records.map((record) => count(record[field]));
    const total = values.reduce((sum, value) => sum + (value ?? 0), 0);
    return [field, values.length && values.every((value) => value !== null) ? count(total) : null];
  }));
  const raw = [totals.input, totals.cached, totals.cacheWrite];
  const inputTotal = raw.every((value) => value !== null) ? count(raw.reduce((a, b) => a + b, 0)) : null;
  return {
    usageRecords: records.length,
    unknownUsageRecords: records.filter((record) => fields.some((field) => count(record[field]) === null)).length,
    ...totals,
    inputTotal,
    readRatio: inputTotal > 0 ? totals.cached / inputTotal : null,
  };
}

// Raw nullable provider counters retain presence information lost during Rust
// normalization. Keep known output even when the input/cache split is unknown.
function attemptUsage(observation) {
  const raw = observation.raw_usage;
  const unknown = () => davinciUsage(null);
  if (!raw || raw.anomalous === true) return unknown();
  const total = count(raw.input_total);
  const cached = count(raw.cache_read);
  const cacheWrite = count(raw.cache_write);
  const output = count(raw.output);
  const cacheTotal = cached !== null && cacheWrite !== null ? count(cached + cacheWrite) : null;
  if (total !== null && ((cached !== null && cached > total) ||
      (cacheWrite !== null && cacheWrite > total) || (cacheTotal !== null && cacheTotal > total))) {
    return unknown();
  }
  const input = total !== null && cacheTotal !== null ? total - cacheTotal : null;
  // Complete raw buckets with a negative reconciliation result are inconsistent,
  // not a partial observation whose known fields can safely be kept.
  if (observation.usage_complete === false && [input, cached, cacheWrite, output].every((value) => value !== null)) {
    return unknown();
  }
  return { input, cached, cacheWrite, output };
}

export function usageFromEvents(harness, lines) {
  const records = [];
  const attempts = new Map();
  let providerEvidence = false;
  let unidentified = 0;
  for (const line of lines) {
    let event;
    try { event = JSON.parse(line); } catch { continue; }
    if (harness === "codex" && event?.type === "turn.completed") {
      records.push(codexUsage(event.usage));
    } else if (harness === "davinci" && event?.type === "message_end" && event.message?.role === "assistant") {
      // Older transcripts cannot prove that normalized zeroes were measured.
      records.push(davinciUsage(null));
    } else if (harness === "davinci" && event?.type === "provider_observation") {
      providerEvidence = true;
      const observation = event.observation;
      if (!["attempt_start", "attempt_end", "telemetry_overflow"].includes(observation?.kind)) continue;
      if (observation.kind === "telemetry_overflow") {
        // Overflow may reuse an already settled attempt's identifiers. It is
        // evidence of missing receipts, not a duplicate of that settled attempt.
        attempts.set(`overflow:${unidentified++}`, { usage: davinciUsage(null), terminal: false });
        continue;
      }
      const identified = typeof observation.logical_request_id === "string" && observation.logical_request_id.length > 0 &&
        count(observation.attempt_id) !== null && observation.attempt_id > 0;
      const key = identified ? JSON.stringify([observation.root_id ?? null, observation.actor_id ?? null,
        observation.logical_request_id, observation.attempt_id]) : `unidentified:${unidentified++}`;
      const previous = attempts.get(key);
      if (observation.kind !== "attempt_end" || !identified) {
        if (!previous) attempts.set(key, { usage: davinciUsage(null), terminal: false });
        continue;
      }
      const usage = attemptUsage(observation);
      if (previous?.terminal && JSON.stringify(previous.usage) !== JSON.stringify(usage)) {
        attempts.set(key, { usage: davinciUsage(null), terminal: true, conflicting: true });
      } else if (!previous?.conflicting) {
        attempts.set(key, { usage, terminal: true });
      }
    }
  }
  if (harness === "davinci" && providerEvidence) {
    // Physical attempt receipts, including retries/host calls, are authoritative
    // for this stream. Never also sum their logical rollups or assistant copies.
    const values = [...attempts.values()].map((attempt) => attempt.usage);
    return { ...summarizeUsage(values.length ? values : [davinciUsage(null)]), usageSource: "provider_attempts" };
  }
  return { ...summarizeUsage(records), usageSource: harness === "codex" ? "codex_turns" : "legacy_unverified" };
}

export function summarizeRuns(rows) {
  const groups = new Map();
  for (const row of rows) {
    const key = JSON.stringify([row.harness, row.task]);
    groups.set(key, [...(groups.get(key) ?? []), row]);
  }
  return [...groups.entries()].map(([key, group]) => {
    const [harness, task] = JSON.parse(key);
    const mean = (field) => group.every((row) => count(row[field]) !== null)
      ? group.reduce((sum, row) => sum + row[field] / group.length, 0) : null;
    return {
      harness, task, runs: group.length,
      input: mean("input"), cached: mean("cached"), cacheWrite: mean("cacheWrite"), output: mean("output"),
      wallMs: mean("wallMs"),
      runsWithUnknownUsage: group.filter((row) => row.inputTotal == null || row.output == null).length,
      passRate: group.filter((row) => row.passed).length / group.length,
    };
  });
}
