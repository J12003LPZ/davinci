'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const {createBrowserBackend, loadTrustedPlaywright} = require('./browser_backend.js');

async function snapshotUntil(session, marker) {
  const deadline = performance.now() + 3000;
  while (performance.now() < deadline) {
    const snapshot = await session.execute({action: 'snapshot'});
    if (snapshot.html.includes(marker)) return snapshot;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  throw new Error('Frontend fixture did not reach its expected state');
}

test('actual browser backend detects planted login failure and verifies the corrected flow', {
  skip: !process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH,
}, async t => {
  const playwright = loadTrustedPlaywright({packagePath: process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH,
    version: '1.62.0', workspace: process.cwd()});
  let launches = 0;
  const backend = createBrowserBackend({chromium: {launch: options => {
    launches++; return playwright.chromium.launch(options);
  }}});
  t.after(() => backend.close());
  const server = http.createServer((req, res) => {
    if (req.url === '/broken-api') {res.writeHead(500); res.end('planted failure'); return;}
    if (req.url === '/login-api') {res.setHeader('set-cookie', 'fixture_login=yes; SameSite=Strict'); res.end('ok'); return;}
    const broken = req.url === '/broken';
    const loggedIn = req.headers.cookie?.includes('fixture_login=yes');
    res.end(`<html><body><label>Email<input aria-label="Email"></label>
      <select data-testid="tenant"><option value="one">One</option><option value="two">Two</option></select>
      <button onclick="login()">Log in</button><p id="status">${loggedIn ? 'Already logged in' : 'Logged out'}</p>
      <script>async function login() {
        ${broken ? "console.error('Planted login console error');" : ''}
        const response = await fetch('${broken ? '/broken-api' : '/login-api'}');
        document.querySelector('#status').textContent = ${broken ? "'Failure observed'" : "'Logged in successfully'"};
      }</script></body></html>`);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  t.after(() => {server.closeAllConnections(); return new Promise(resolve => server.close(resolve));});
  const broken = await backend.open({origins: [origin]});
  t.after(() => broken.close());
  await broken.execute({action: 'navigate', url: `${origin}/broken`});
  await broken.execute({action: 'click', selector: {kind: 'role', role: 'button', name: 'Log in'}});
  const failed = await snapshotUntil(broken, '<p id="status">Failure observed</p>');
  assert.ok(!failed.html.includes('<p id="status">Logged in successfully</p>'));
  const failedConsole = await broken.execute({action: 'console'});
  const failedNetwork = await broken.execute({action: 'network'});
  assert.ok(failedConsole.events.some(event => event.message.includes('Planted login console error')));
  assert.ok(failedNetwork.events.some(event => event.status === 500));
  assert.equal(broken.evidence().healthy, false);

  const [fixed, isolated] = await Promise.all([backend.open({origins: [origin]}), backend.open({origins: [origin]})]);
  t.after(() => fixed.close()); t.after(() => isolated.close());
  assert.equal(launches, 1);
  await fixed.execute({action: 'navigate', url: origin});
  await fixed.execute({action: 'type', selector: {kind: 'label', value: 'Email'}, text: 'fixture@example.test'});
  await fixed.execute({action: 'select', selector: {kind: 'test_id', value: 'tenant'}, value: 'two'});
  await fixed.execute({action: 'click', selector: {kind: 'role', role: 'button', name: 'Log in'}});
  await snapshotUntil(fixed, '<p id="status">Logged in successfully</p>');
  assert.ok((await fixed.execute({action: 'accessibility'})).tree.includes('Log in'));
  const screenshot = await fixed.execute({action: 'screenshot'});
  assert.equal(screenshot.mediaType, 'image/png');
  assert.ok(Buffer.isBuffer(screenshot.bytes));
  assert.equal(screenshot.bytes.subarray(1, 4).toString(), 'PNG');
  assert.ok(screenshot.bytes.length <= 4 * 1024 * 1024);
  assert.equal(fixed.evidence().healthy, true);
  await fixed.execute({action: 'navigate', url: origin});
  assert.ok((await fixed.execute({action: 'snapshot'})).html.includes('<p id="status">Already logged in</p>'));
  await isolated.execute({action: 'navigate', url: origin});
  assert.ok((await isolated.execute({action: 'snapshot'})).html.includes('<p id="status">Logged out</p>'));
  assert.ok(!isolated.browserVersion.includes('fixture'));
});
