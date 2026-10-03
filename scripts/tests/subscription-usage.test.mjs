import assert from "node:assert/strict";
import { test } from "node:test";
import { codexUsage, davinciUsage, summarizeUsage, summarizeRuns, usageFromEvents } from "../subscription-usage.mjs";

test("Codex and DaVinci observations use the same disjoint token buckets", () => {
  const codex = codexUsage({ input_tokens: 100, cached_input_tokens: 75, output_tokens: 12 });
  const davinci = davinciUsage({ input: 25, cacheRead: 75, cacheWrite: 0, output: 12 });
  assert.deepEqual(codex, davinci);
  assert.equal(summarizeUsage([codex]).readRatio, 0.75);
});

test("missing and partially missing usage remain unknown instead of zero", () => {
  assert.equal(summarizeUsage([]).input, null);
  assert.equal(summarizeUsage([]).readRatio, null);
  const summary = summarizeUsage([codexUsage({ input_tokens: 100, output_tokens: 12 })]);
  assert.equal(summary.input, null);
  assert.equal(summary.cached, null);
  assert.equal(summary.output, 12);
  assert.equal(summary.unknownUsageRecords, 1);
  const mixed = summarizeUsage([davinciUsage({ input: 4, cacheRead: 0, cacheWrite: 0, output: 2 }), davinciUsage(null)]);
  assert.equal(mixed.input, null);
  assert.equal(mixed.usageRecords, 2);
});

test("malformed counts, impossible cache counts and overflow are not savings", () => {
  for (const value of [-1, "10", 1.5, Infinity, NaN]) {
    assert.equal(davinciUsage({ input: value }).input, null);
  }
  assert.equal(codexUsage({ input_tokens: 2, cached_input_tokens: 3 }).cached, null);
  const huge = { input: Number.MAX_SAFE_INTEGER, cached: 0, cacheWrite: 0, output: 0 };
  assert.equal(summarizeUsage([huge, huge]).input, null);
  assert.equal(summarizeUsage([{ input: 0, cached: 0, cacheWrite: 0, output: 0 }]).readRatio, null);
});

test("only terminal usage observations are counted and API cost is ignored", () => {
  const summary = usageFromEvents("davinci", [
    "invalid JSON", "null",
    JSON.stringify({ type: "message_update", message: { role: "assistant", usage: { input: 999 } } }),
    JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { input: 20, cacheRead: 30, cacheWrite: 5, output: 7, cost: { total: 999 } } } }),
    JSON.stringify({ type: "message_end", message: { role: "user" } }),
  ]);
  assert.equal(summary.usageRecords, 1);
  assert.equal(summary.inputTotal, 55);
  assert.equal(summary.readRatio, 30 / 55);
  assert.equal("cost" in summary, false);
  assert.equal("credits" in summary, false);
  const missing = usageFromEvents("codex", [JSON.stringify({ type: "turn.completed" })]);
  assert.equal(missing.unknownUsageRecords, 1);
  assert.equal(missing.output, null);
});

test("summaries retain failures and never average missing usage as zero", () => {
  const rows = [
    { harness: "davinci", task: "fix", input: 100, cached: 20, cacheWrite: 0, inputTotal: 120, output: 10, wallMs: 1000, passed: true },
    { harness: "davinci", task: "fix", input: null, cached: null, cacheWrite: null, inputTotal: null, output: null, wallMs: 2000, passed: false },
  ];
  const [summary] = summarizeRuns(rows);
  assert.equal(summary.input, null);
  assert.equal(summary.output, null);
  assert.equal(summary.wallMs, 1500);
  assert.equal(summary.passRate, 0.5);
  assert.equal(summary.runsWithUnknownUsage, 1);
  assert.equal(summary.runs, 2);
});
