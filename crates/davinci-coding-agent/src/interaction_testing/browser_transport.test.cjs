'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {PassThrough} = require('node:stream');
const {createBrowserTransport} = require('./browser_transport.js');

function harness(backend, artifact) {
  const input = new PassThrough();
  const output = new PassThrough();
  const responses = [];
  let text = '';
  output.on('data', chunk => {
    text += chunk.toString();
    for (let at; (at = text.indexOf('\n')) >= 0;) {
      responses.push(JSON.parse(text.slice(0, at))); text = text.slice(at + 1);
    }
  });
  const transport = createBrowserTransport({input, output, backend, artifact});
  return {input, responses, transport, send: value => input.write(JSON.stringify(value) + '\n')};
}
const tick = () => new Promise(resolve => setImmediate(resolve));

test('fragments and correlated responses; duplicate IDs fail closed', async () => {
  let closed = 0;
  const h = harness({open: async () => ({browserVersion: 'test', close: async () => {closed++;},
    execute: async command => ({action: command.action})}), close: async () => {}});
  h.input.write('{"id":1,"op":"open","options":');
  h.input.write('{"origins":["http://localhost:3000"]}}\n');
  await tick();
  assert.deepEqual(h.responses[0], {id: 1, ok: true, result: {resource: 1, browserVersion: 'test'}});
  h.send({id: 2, op: 'execute', resource: 1, command: {action: 'snapshot'}});
  await tick();
  assert.equal(h.responses[1].result.action, 'snapshot');
  h.send({id: 2, op: 'close', resource: 1});
  assert.equal((await h.transport.done).failed, true);
  assert.equal(closed, 1);
});

test('oversized and invalid UTF-8 frames fail before backend I/O', async () => {
  for (const chunk of [Buffer.alloc(16385, 0x20), Buffer.from([0xff, 0x0a])]) {
    let opened = 0;
    const h = harness({open: async () => {opened++;}, close: async () => {}});
    h.input.write(chunk);
    assert.equal((await h.transport.done).failed, true);
    assert.equal(opened, 0);
    assert.equal(h.responses[0].error, 'Browser transport protocol failure');
  }
});

test('close interrupts a held action without blocking another resource', async () => {
  let release;
  const held = new Promise((_, reject) => {release = () => reject(new Error('secret URL'));});
  let opened = 0;
  const h = harness({open: async () => {
    const first = ++opened === 1;
    return {browserVersion: 'test', execute: async () => first ? held : {done: true},
      close: async () => {if (first) release();}};
  }, close: async () => {}});
  h.send({id: 1, op: 'open', options: {origins: ['http://localhost:3000']}});
  h.send({id: 2, op: 'open', options: {origins: ['http://localhost:3000']}});
  await tick();
  h.send({id: 3, op: 'execute', resource: 1, command: {action: 'click'}});
  h.send({id: 4, op: 'execute', resource: 2, command: {action: 'snapshot'}});
  h.send({id: 5, op: 'close', resource: 1});
  await tick();
  assert.equal(h.responses.find(r => r.id === 4).ok, true);
  assert.equal(h.responses.find(r => r.id === 5).ok, true);
  assert.equal(h.responses.find(r => r.id === 3).error, 'Browser request failed');
  h.input.end();
  assert.equal((await h.transport.done).failed, false);
});

test('binary screenshot requires host artifact sink; bytes never enter JSONL', async () => {
  let stores = 0;
  const h = harness({open: async () => ({browserVersion: 'test', close: async () => {},
    execute: async () => ({bytes: Buffer.from('private pixels'), mediaType: 'image/png'})}), close: async () => {}},
  async (resource, result) => {stores++; assert.equal(resource, 1); assert.ok(Buffer.isBuffer(result.bytes));
    return {artifact: 'host-reference', size: result.bytes.length, mediaType: result.mediaType};});
  h.send({id: 1, op: 'open', options: {origins: ['http://localhost:3000']}});
  await tick();
  h.send({id: 2, op: 'execute', resource: 1, command: {action: 'screenshot'}});
  await tick();
  assert.equal(stores, 1);
  assert.equal(h.responses[1].result.artifact, 'host-reference');
  assert.ok(!JSON.stringify(h.responses).includes('private pixels'));
  h.input.end(); await h.transport.done;
});

test('unknown fields, truncated EOF and oversized response cannot succeed', async () => {
  for (const frame of ['{"id":1,"op":"shutdown","evaluate":"x"}\n', '{"id":1']) {
    const h = harness({close: async () => {}});
    h.input.end(frame);
    assert.equal((await h.transport.done).failed, true);
  }
  const h = harness({open: async () => ({browserVersion: 'test', close: async () => {},
    execute: async () => ({html: '\u0000'.repeat(48000)})}), close: async () => {}});
  h.send({id: 1, op: 'open', options: {origins: ['http://localhost:3000']}});
  await tick();
  h.send({id: 2, op: 'execute', resource: 1, command: {action: 'snapshot'}});
  await tick();
  assert.equal(h.responses[1].ok, false);
  h.input.end(); await h.transport.done;
});

test('shutdown cancels startup and cleanup failure cannot yield success', async () => {
  let cancelled = false;
  const h = harness({open: async (_, signal) => new Promise((_, reject) => {
    signal.addEventListener('abort', () => {cancelled = true; reject(new Error('cancelled'));}, {once: true});
  }), close: async () => {throw new Error('cleanup failed');}});
  h.send({id: 1, op: 'open', options: {origins: ['http://localhost:3000']}});
  h.send({id: 2, op: 'shutdown'});
  assert.equal((await h.transport.done).failed, true);
  assert.equal(cancelled, true);
  assert.equal(h.responses.find(r => r.id === 1).ok, false);
  assert.equal(h.responses.find(r => r.id === 2).error, 'Browser cleanup failed');
});

test('pending admission is bounded before a seventeenth backend operation', async () => {
  let opened = 0;
  const h = harness({open: async (_, signal) => {
    opened++;
    return new Promise((_, reject) => signal.addEventListener('abort', () => reject(new Error('cancelled')), {once: true}));
  }, close: async () => {}});
  for (let id = 1; id <= 17; id++) h.send({id, op: 'open', options: {origins: ['http://localhost:3000']}});
  assert.equal((await h.transport.done).failed, true);
  assert.equal(opened, 16);
});

test('missing artifact sink rejects screenshot before backend execution', async () => {
  let executed = 0;
  const h = harness({open: async () => ({browserVersion: 'test', close: async () => {},
    execute: async () => {executed++;}}), close: async () => {}});
  h.send({id: 1, op: 'open', options: {origins: ['http://localhost:3000']}});
  await tick();
  h.send({id: 2, op: 'execute', resource: 1, command: {action: 'screenshot'}});
  await tick();
  assert.equal(executed, 0);
  assert.equal(h.responses[1].ok, false);
  h.input.end(); await h.transport.done;
});

test('output backpressure keeps pending admission bounded', async () => {
  const input = new PassThrough();
  const output = new PassThrough({highWaterMark: 1});
  let opened = 0;
  const transport = createBrowserTransport({input, output, backend: {
    open: async () => {opened++; return {browserVersion: 'test', close: async () => {}};}, close: async () => {},
  }});
  for (let id = 1; id <= 17; id++) input.write(JSON.stringify({id, op: 'open', options: {origins: ['http://localhost:3000']}}) + '\n');
  await tick();
  assert.equal(opened, 16);
  assert.ok(output.writableLength < 16 * 65536);
  output.resume();
  assert.equal((await transport.done).failed, true);
});
