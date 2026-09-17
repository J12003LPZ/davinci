'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const {createOriginProxy} = require('./browser_network.js');

async function listen(server) {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return `http://127.0.0.1:${server.address().port}`;
}

// The test operator supplies the trusted installed package explicitly. Never
// resolve Playwright from the repository's node_modules or install at runtime.
test('real Chromium enforces the proxy on redirects and subresources', {
  skip: !process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH,
}, async t => {
  const playwright = require(process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH);
  let forbiddenHits = 0;
  const forbidden = http.createServer((_req, res) => {forbiddenHits++; res.end('forbidden');});
  const destination = await listen(forbidden);
  t.after(() => {forbidden.closeAllConnections(); return new Promise(resolve => forbidden.close(resolve));});
  let origin;
  const source = http.createServer((req, res) => {
    if (req.url === '/external') res.writeHead(302, {location: `${destination}/target`});
    else if (req.url === '/internal') res.writeHead(302, {location: `${origin}/target`});
    else {
      res.end(`<html><body>owned fixture<img src="${destination}/image"></body></html>`);
      return;
    }
    res.end();
  });
  origin = await listen(source);
  t.after(() => {source.closeAllConnections(); return new Promise(resolve => source.close(resolve));});
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const browser = await playwright.chromium.launch({headless: true, timeout: 15000});
  t.after(() => browser.close());
  const context = await browser.newContext({
    serviceWorkers: 'block', proxy: {server: proxy.serverUrl, bypass: '<-loopback>'},
  });
  t.after(() => context.close());
  const page = await context.newPage();
  assert.equal((await page.goto(`${origin}/external`, {timeout: 5000})).status(), 403);
  assert.equal(forbiddenHits, 0);
  assert.equal((await page.goto(`${origin}/internal`, {timeout: 5000, waitUntil: 'load'})).status(), 200);
  assert.equal(page.url(), `${origin}/target`);
  assert.equal(await page.locator('body').innerText(), 'owned fixture');
  assert.equal(forbiddenHits, 0);
  assert.ok(proxy.metrics().denied >= 2);
});
