import assert from 'node:assert/strict';
import test from 'node:test';
import { usageFromEvents } from '../subscription-usage.mjs';

const usage = (input = 100, output = 10) => ({ input, cacheRead: 20, cacheWrite: 0, output });
const assistant = (value = usage()) => ({ type: 'message_end', message: { role: 'assistant', usage: value } });
const observation = (overrides = {}) => ({ type: 'provider_observation', observation: {
  schema_version: 1, kind: 'attempt_end', logical_request_id: 'request-1', attempt_id: 1,
  status: 'completed', usage_complete: true, usage: usage(),
  raw_usage: { input_total: 120, cache_read: 20, cache_write: 0, output: 10, anomalous: false },
  ...overrides,
} });
const summarize = (...events) => usageFromEvents('davinci', events.map(JSON.stringify));

test('complete receipt and assistant message are counted exactly once', () => {
  const value = summarize(observation(), assistant());
  assert.equal(value.usageRecords, 1);
  assert.equal(value.inputTotal, 120);
  assert.equal(value.output, 10);
  assert.equal(value.unknownUsageRecords, 0);
});
test('partial receipt overrides zero-filled assistant counters', () => {
  const zeroFilled = { input: 100, output: 0, cacheRead: 0, cacheWrite: 0 };
  const value = summarize(observation({ usage_complete: false, raw_usage: null, usage: zeroFilled }), assistant(zeroFilled));
  assert.equal(value.output, null);
  assert.equal(value.unknownUsageRecords, 1);
});
test('receipt-only host operations are included', () => {
  const value = summarize(observation());
  assert.equal(value.inputTotal, 120);
  assert.equal(value.output, 10);
});
test('retries are distinct attempts, but repeated terminal events are not', () => {
  const failed = observation({ status: 'failed' });
  const completed = observation({ attempt_id: 2 });
  const value = summarize(failed, completed, completed, assistant());
  assert.equal(value.usageRecords, 2);
  assert.equal(value.inputTotal, 240);
  assert.equal(value.output, 20);
});
test('an unfinished attempt is unknown even with a complete later retry', () => {
  const value = summarize(observation({ kind: 'attempt_start', usage: null, usage_complete: null, raw_usage: null }), observation({ attempt_id: 2 }));
  assert.equal(value.usageRecords, 2);
  assert.equal(value.output, null);
  assert.equal(value.unknownUsageRecords, 1);
});
test('legitimate measured zeros stay zero', () => {
  const zero = { input: 0, cacheRead: 0, cacheWrite: 0, output: 0 };
  const value = summarize(observation({ usage: zero, raw_usage: { input_total: 0, cache_read: 0, cache_write: 0, output: 0, anomalous: false } }));
  assert.equal(value.output, 0);
  assert.equal(value.inputTotal, 0);
  assert.equal(value.unknownUsageRecords, 0);
});
test('legacy assistant-only counters cannot establish completeness', () => {
  const value = summarize(assistant());
  assert.equal(value.output, null);
  assert.equal(value.unknownUsageRecords, 1);
});
test('a missing completeness flag remains unknown', () => {
  assert.equal(summarize(observation({ usage_complete: null }), assistant()).output, null);
});
test('conflicting duplicate receipts fail closed', () => {
  assert.equal(summarize(observation(), observation({ usage_complete: false })).output, null);
});
test('raw and normalized counter disagreement fails closed', () => {
  assert.equal(summarize(observation({ usage: usage(101) })).inputTotal, null);
});
test('observation with no complete raw evidence is unknown', () => {
  assert.equal(summarize(observation({ raw_usage: null })).output, null);
});
test('different logical requests may reuse an attempt number', () => {
  const value = summarize(observation(), observation({ logical_request_id: 'request-2' }));
  assert.equal(value.usageRecords, 2);
  assert.equal(value.output, 20);
});
test('invalid attempt identities cannot look measured', () => {
  const value = summarize(observation({ logical_request_id: '' }));
  assert.equal(value.unknownUsageRecords, 1);
  assert.equal(value.output, null);
});
test('telemetry overflow makes the run incomplete', () => {
  const value = summarize(observation(), observation({ kind: 'telemetry_overflow', status: 'unknown' }));
  assert.equal(value.output, null);
  assert.ok(value.unknownUsageRecords > 0);
});
test('Codex terminal counters keep their existing behavior', () => {
  const value = usageFromEvents('codex', [JSON.stringify({ type: 'turn.completed', usage: { input_tokens: 120, cached_input_tokens: 20, output_tokens: 10 } })]);
  assert.equal(value.inputTotal, 120);
  assert.equal(value.output, 10);
});
test('out-of-order starts do not erase a terminal receipt', () => {
  assert.equal(summarize(observation(), observation({ kind: 'attempt_start' })).output, 10);
});
test('invalid terminal status never looks complete', () => {
  assert.equal(summarize(observation({ status: 'running' })).output, null);
});
test('non-JSON diagnostics and null records are ignored', () => {
  const value = usageFromEvents('davinci', ['hello', 'null', JSON.stringify(observation())]);
  assert.equal(value.output, 10);
});
