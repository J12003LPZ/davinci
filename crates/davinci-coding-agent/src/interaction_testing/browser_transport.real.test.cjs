'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const {PassThrough} = require('node:stream');
const {createHash} = require('node:crypto');
const {createBrowserBackend, loadTrustedPlaywright} = require('./browser_backend.js');
const {createBrowserTransport} = require('./browser_transport.js');

test('actual Chromium actions and PNG cross bounded correlated JSONL without binary output', {
  skip: !process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH,
}, async t => {
  const backend = createBrowserBackend(loadTrustedPlaywright({packagePath: process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH,
    version: '1.62.0', workspace: process.cwd()}));
  const server = http.createServer((_, response) => response.end('<html><body><button onclick="this.textContent=\'Done\'">Start</button></body></html>'));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => {server.closeAllConnections(); return new Promise(resolve => server.close(resolve));});
  const origin = `http://127.0.0.1:${server.address().port}`;
  const input = new PassThrough();
  const output = new PassThrough();
  const waiters = new Map();
  let frame = '';
  let retained;
  const transport = createBrowserTransport({input, output, backend,
    artifact: async (resource, result) => {
      assert.equal(resource, 1);
      retained = result.bytes;
      return {artifact: `fixture:${createHash('sha256').update(retained).digest('hex')}`, size: retained.length, mediaType: 'image/png'};
    }});
  t.after(() => transport.close());
  output.on('data', chunk => {
    frame += chunk.toString();
    for (let end; (end = frame.indexOf('\n')) >= 0;) {
      const line = frame.slice(0, end); frame = frame.slice(end + 1);
      assert.ok(Buffer.byteLength(line) + 1 <= 65536);
      const response = JSON.parse(line);
      const waiter = waiters.get(response.id);
      assert.ok(waiter); waiters.delete(response.id); waiter(response);
    }
  });
  let sequence = 0;
  async function request(op, values = {}) {
    const id = ++sequence;
    const response = await new Promise(resolve => {
      waiters.set(id, resolve); input.write(JSON.stringify({id, op, ...values}) + '\n');
    });
    assert.equal(response.ok, true, JSON.stringify(response));
    return response.result;
  }
  const opened = await request('open', {options: {origins: [origin]}});
  assert.ok(!opened.browserVersion.includes('fixture'));
  await request('execute', {resource: 1, command: {action: 'navigate', url: origin}});
  await request('execute', {resource: 1, command: {action: 'click', selector: {kind: 'role', role: 'button', name: 'Start'}}});
  assert.ok((await request('execute', {resource: 1, command: {action: 'snapshot'}})).html.includes('>Done</button>'));
  const screenshot = await request('execute', {resource: 1, command: {action: 'screenshot'}});
  assert.equal(retained.subarray(1, 4).toString(), 'PNG');
  assert.equal(screenshot.size, retained.length);
  assert.ok(!JSON.stringify(screenshot).includes('bytes'));
  await request('close', {resource: 1});
  await request('shutdown');
  assert.equal((await transport.done).failed, false);
});
