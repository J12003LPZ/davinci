import assert from "node:assert/strict";
import { test } from "node:test";
import { usageFromEvents } from "../subscription-usage.mjs";

const usage = { input: 100, cacheRead: 0, cacheWrite: 0, output: 0 };
const message = { type: "message_end", message: { role: "assistant", usage } };
const receipt = (fields = {}) => ({ type: "provider_observation", observation: {
  logical_request_id: "review", attempt_id: 1, kind: "attempt_end", status: "completed",
  ...fields,
} });
const summarize = (...events) => usageFromEvents("davinci", events.map(JSON.stringify));
const raw = { input_total: 100, cache_read: 50, cache_write: 0, output: 8 };

test("partial provider counters never become measured normalized zeros", () => {
  const result = summarize(receipt({ usage, usage_complete: false, raw_usage: { input_total: 100 } }), message);
  assert.equal(result.output, null);
  assert.equal(result.cached, null);
  assert.equal(result.input, null);
  assert.equal(result.unknownUsageRecords, 1);
  assert.equal(result.usageSource, "provider_attempts");
});

test("missing raw receipts and legacy transcript counters remain unverified", () => {
  for (const result of [summarize(message), summarize(receipt({ usage, usage_complete: false }), message)]) {
    assert.equal(result.output, null);
    assert.equal(result.unknownUsageRecords, 1);
  }
  assert.equal(summarize(message).usageSource, "legacy_unverified");
});

test("attempt receipts include retries and host calls without counting transcript and logical rollups", () => {
  const one = receipt({ raw_usage: raw, usage_complete: true });
  const two = receipt({ attempt_id: 2, raw_usage: { ...raw, output: 3 }, usage_complete: true });
  const host = receipt({ logical_request_id: "design", raw_usage: { ...raw, output: 2 }, usage_complete: true });
  const result = summarize(one, two, one, message, host, receipt({ kind: "logical_end", raw_usage: raw }));
  assert.equal(result.usageRecords, 3);
  assert.equal(result.output, 13);
  assert.equal(result.input, 150);
  assert.equal(result.cached, 150);
});

test("unfinished attempts, malformed counts and conflicting duplicate receipts stay unknown", () => {
  const one = receipt({ raw_usage: raw, usage_complete: true });
  for (const other of [
    receipt({ attempt_id: 2, kind: "attempt_start" }),
    receipt({ attempt_id: 2, raw_usage: { ...raw, output: -1 } }),
    receipt({ raw_usage: { ...raw, output: 9 }, usage_complete: true }),
  ]) {
    const result = summarize(one, other, one);
    assert.equal(result.output, null);
    assert.equal(result.unknownUsageRecords, 1);
  }
});

test("known output survives missing cache details but anomalies do not", () => {
  const partial = summarize(receipt({ raw_usage: { input_total: 100, output: 7 }, usage_complete: false }));
  assert.equal(partial.output, 7);
  assert.equal(partial.inputTotal, null);
  for (const invalid of [
    { ...raw, cache_read: 101 },
    { ...raw, anomalous: true },
    { ...raw, input_total: Number.MAX_SAFE_INTEGER + 1 },
  ]) {
    assert.equal(summarize(receipt({ raw_usage: invalid })).input, null);
  }
  assert.equal(summarize(receipt({ raw_usage: raw, usage_complete: false })).output, null);
});

test("explicit provider zero and Codex CLI zero are valid observations", () => {
  const result = summarize(receipt({ raw_usage: { input_total: 0, cache_read: 0, cache_write: 0, output: 0 }, usage_complete: true }));
  assert.equal(result.output, 0);
  assert.equal(result.unknownUsageRecords, 0);
  const codex = usageFromEvents("codex", [JSON.stringify({ type: "turn.completed", usage: { input_tokens: 0, cached_input_tokens: 0, output_tokens: 0 } })]);
  assert.equal(codex.output, 0);
  assert.equal(codex.unknownUsageRecords, 0);
});


test("cache measurement CLI does not advertise legacy normalized zero as observed usage", async () => {
  const { mkdtempSync, writeFileSync, rmSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { spawnSync } = await import("node:child_process");
  const { fileURLToPath } = await import("node:url");
  const dir = mkdtempSync(join(tmpdir(), "davinci-usage-review-"));
  try {
    const path = join(dir, "session.jsonl");
    writeFileSync(path, JSON.stringify({ type: "message", message: message.message }) + "\n");
    const result = spawnSync(process.execPath, [fileURLToPath(new URL("../measure-codex-cache.mjs", import.meta.url)), path], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /source=legacy_unverified/);
    assert.match(result.stdout, /output=unknown/);
    assert.match(result.stdout, /fresh=unknown/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
