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

// Normalize only receipts with independent evidence that no counter was filled
// in by the decoder. A real, explicitly measured zero is still a measurement.
function measuredAttemptUsage(observation) {
  const raw = observation.raw_usage;
  const normalized = davinciUsage(observation.usage);
  const fields = [normalized.input, normalized.cached, normalized.cacheWrite, normalized.output];
  const rawFields = [raw?.input_total, raw?.cache_read, raw?.cache_write, raw?.output];
  if (observation.usage_complete !== true
      || !["completed", "failed", "cancelled"].includes(observation.status)
      || raw?.anomalous !== false
      || [...fields, ...rawFields].some((value) => count(value) === null)
      || count(normalized.input + normalized.cached + normalized.cacheWrite) !== raw.input_total
      || normalized.cached !== raw.cache_read
      || normalized.cacheWrite !== raw.cache_write
      || normalized.output !== raw.output) {
    return davinciUsage(null);
  }
  return normalized;
}

export function usageFromEvents(harness, lines) {
  const records = [];
  const attempts = new Map();
  let assistantRecords = 0;
  let incompleteObservations = 0;
  for (const line of lines) {
    let event;
    try { event = JSON.parse(line); } catch { continue; }
    if (harness === "codex" && event?.type === "turn.completed") {
      records.push(codexUsage(event.usage));
      continue;
    }
    if (harness !== "davinci") continue;
    if (event?.type === "message_end" && event.message?.role === "assistant") {
      assistantRecords += 1;
    }
    if (event?.type !== "provider_observation") continue;
    const observation = event.observation;
    if (observation?.kind === "telemetry_overflow") {
      incompleteObservations += 1;
      continue;
    }
    if (!["attempt_start", "attempt_end"].includes(observation?.kind)) continue;
    const request = observation.logical_request_id;
    const attempt = count(observation.attempt_id);
    if (typeof request !== "string" || !request || attempt === null) {
      incompleteObservations += 1;
      continue;
    }
    // JSON tuple encoding avoids delimiter collisions between request IDs.
    const key = JSON.stringify([request, attempt]);
    const previous = attempts.get(key);
    if (observation.kind === "attempt_start") {
      if (!previous) attempts.set(key, { terminal: false, usage: davinciUsage(null) });
      continue;
    }
    const usage = measuredAttemptUsage(observation);
    // Compare only accounting evidence; elapsed time and object-key order must
    // not turn a repeated serialization into another billable attempt.
    const fingerprint = JSON.stringify([
      observation.status, observation.usage_complete,
      usage.input, usage.cached, usage.cacheWrite, usage.output,
    ]);
    const conflicted = previous?.conflicted
      || (previous?.terminal && previous.fingerprint !== fingerprint);
    attempts.set(key, {
      terminal: true,
      fingerprint,
      conflicted,
      usage: conflicted ? davinciUsage(null) : usage,
    });
  }
  if (harness === "davinci") {
    if (attempts.size || incompleteObservations) {
      records.push(...[...attempts.values()].map((attempt) => attempt.usage));
      for (let i = 0; i < incompleteObservations; i += 1) records.push(davinciUsage(null));
    } else {
      // Old message_end counters have no field-presence evidence. Do not
      // present decoder defaults as measured usage, even when they are zero.
      for (let i = 0; i < assistantRecords; i += 1) records.push(davinciUsage(null));
    }
  }
  return summarizeUsage(records);
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
