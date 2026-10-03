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

export function usageFromEvents(harness, lines) {
  const records = [];
  for (const line of lines) {
    let event;
    try { event = JSON.parse(line); } catch { continue; }
    if (harness === "codex" && event?.type === "turn.completed") {
      records.push(codexUsage(event.usage));
    } else if (harness === "davinci" && event?.type === "message_end" && event.message?.role === "assistant") {
      records.push(davinciUsage(event.message.usage));
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
