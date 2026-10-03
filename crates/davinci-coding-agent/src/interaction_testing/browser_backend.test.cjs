'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {createBrowserBackend, loadTrustedPlaywright} = require('./browser_backend.js');

function fixture() {
  const counts = {launches: 0, contexts: 0, closes: 0, pageHandlers: new Map()};
  const playwright = {chromium: {async launch() {
    counts.launches++;
    return {version: () => 'fixture', async close() {counts.closes++;}, async newContext() {
      counts.contexts++;
      const handlers = new Map();
      return {on(name, handler) {handlers.set(name, handler);}, async route() {}, async routeWebSocket() {}, async close() {},
        async newPage() {
          const page = {on(name, handler) {counts.pageHandlers.set(name, handler);}, setDefaultTimeout() {}};
          handlers.get('page')?.(page); return page;
        }};
    }};
  }}};
  return {counts, playwright};
}

test('browser setup stays lazy and invalid origins or viewport do not launch', async () => {
  const {counts, playwright} = fixture();
  const backend = createBrowserBackend(playwright);
  assert.equal(counts.launches, 0);
  await assert.rejects(backend.open({origins: ['file:///private']}));
  await assert.rejects(backend.open({origins: ['http://127.0.0.1:1'], viewport: {width: 100000, height: 10}}));
  assert.equal(counts.launches, 0);
  await backend.close();
});

test('simultaneous browser opens share one engine and isolate contexts', async () => {
  const {counts, playwright} = fixture();
  const backend = createBrowserBackend(playwright);
  const sessions = await Promise.all([backend.open({origins: ['http://127.0.0.1:1']}),
    backend.open({origins: ['http://127.0.0.1:2']})]);
  assert.equal(counts.launches, 1);
  assert.equal(counts.contexts, 2);
  assert.notEqual(sessions[0], sessions[1]);
  await backend.close(); await backend.close();
  assert.equal(counts.closes, 1);
});

test('browser actions reject arbitrary protocols, unknown fields and unsupported scripts', async () => {
  const {playwright} = fixture();
  const backend = createBrowserBackend(playwright);
  const session = await backend.open({origins: ['http://127.0.0.1:1']});
  try {
    for (const command of [{action: 'navigate', url: 'file:///private'}, {action: 'navigate', url: 'ws://127.0.0.1:1'},
      {action: 'navigate', url: 'http://127.0.0.1:2'}, {action: 'evaluate', script: 'fixture'},
      {action: 'snapshot', extra: true}, {action: 'click', selector: {kind: 'script', value: 'fixture'}}]) {
      await assert.rejects(session.execute(command));
    }
  } finally {await backend.close();}
});

test('cancelled opens never launch and trusted package loader refuses relative paths', async () => {
  const {counts, playwright} = fixture();
  const backend = createBrowserBackend(playwright);
  const controller = new AbortController(); controller.abort();
  await assert.rejects(backend.open({origins: ['http://127.0.0.1:1']}, controller.signal));
  assert.equal(counts.launches, 0);
  assert.throws(() => loadTrustedPlaywright({packagePath: 'node_modules/playwright', version: '1.62.0', workspace: process.cwd()}));
  await backend.close();
});

test('shutdown during shared engine startup rejects the open and closes the engine', async () => {
  let release;
  let started;
  const launched = new Promise(resolve => {started = resolve;});
  const gate = new Promise(resolve => {release = resolve;});
  let closes = 0;
  const backend = createBrowserBackend({chromium: {async launch() {
    started(); await gate;
    return {async close() {closes++;}, async newContext() {throw new Error('Context must not start after shutdown');}};
  }}});
  const open = backend.open({origins: ['http://127.0.0.1:1']});
  const rejected = assert.rejects(open, /cancelled/);
  await launched;
  const shutdown = backend.close();
  release(); await rejected; await shutdown;
  assert.equal(closes, 1);
});

test('aggregate telemetry overflow is bounded and cannot produce healthy evidence', async () => {
  const {counts, playwright} = fixture();
  const backend = createBrowserBackend(playwright);
  const session = await backend.open({origins: ['http://127.0.0.1:1']});
  try {
    for (let index = 0; index < 256; index++) {
      counts.pageHandlers.get('console')({type: () => 'error', text: () => '\u{1f642}'.repeat(1024)});
    }
    const evidence = session.evidence();
    assert.equal(evidence.omitted, true);
    assert.equal(evidence.healthy, false);
    assert.ok(evidence.events.length <= 128);
    assert.ok(Buffer.byteLength(JSON.stringify(evidence)) <= 64 * 1024);
  } finally {await backend.close();}
});
