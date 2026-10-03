'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const https = require('node:https');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {createHash} = require('node:crypto');
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

test('real Chromium uses authorized HTTPS and WSS while refusing a foreign redirect', {
  skip: !process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH || !process.env.DAVINCI_TEST_OPENSSL_PATH,
}, async t => {
  const playwright = require(process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH);
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'davinci-browser-tls-test-'));
  t.after(() => fs.rmSync(fixture, {recursive: true}));
  const keyPath = path.join(fixture, 'fixture-key.pem');
  const certPath = path.join(fixture, 'fixture-cert.pem');
  execFileSync(process.env.DAVINCI_TEST_OPENSSL_PATH, ['req', '-x509', '-newkey', 'rsa:2048',
    '-nodes', '-keyout', keyPath, '-out', certPath, '-days', '1', '-subj', '/CN=localhost'],
    {stdio: 'ignore', timeout: 10000});
  let forbiddenHits = 0;
  const forbidden = http.createServer((_req, res) => {forbiddenHits++; res.end();});
  const destination = await listen(forbidden);
  t.after(() => {forbidden.closeAllConnections(); return new Promise(resolve => forbidden.close(resolve));});
  let origin;
  const peers = new Set();
  const source = https.createServer({key: fs.readFileSync(keyPath), cert: fs.readFileSync(certPath)}, (req, res) => {
    if (req.url === '/external') {res.writeHead(302, {location: destination}); res.end(); return;}
    res.end(`<html><body><p id="secure">pending</p><script>
      const socket = new WebSocket('${origin.replace('https:', 'wss:')}/socket');
      socket.onmessage = event => document.querySelector('#secure').textContent = event.data;
    </script></body></html>`);
  });
  source.on('upgrade', (req, socket) => {
    peers.add(socket); socket.once('close', () => peers.delete(socket)); socket.on('error', () => {});
    const accept = createHash('sha1').update(req.headers['sec-websocket-key'] +
      '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const text = Buffer.from('owned secure websocket');
    socket.write(Buffer.concat([Buffer.from([0x81, text.length]), text]));
  });
  origin = (await listen(source)).replace('http:', 'https:');
  t.after(() => {
    for (const socket of peers) socket.destroy();
    source.closeAllConnections(); return new Promise(resolve => source.close(resolve));
  });
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const browser = await playwright.chromium.launch({headless: true, timeout: 15000});
  t.after(() => browser.close());
  // This generated self-signed fixture explicitly opts out of certificate
  // validation; this option is not a production bridge default.
  const context = await browser.newContext({serviceWorkers: 'block', ignoreHTTPSErrors: true,
    proxy: {server: proxy.serverUrl, bypass: '<-loopback>'}});
  t.after(() => context.close());
  const page = await context.newPage();
  assert.equal((await page.goto(`${origin}/external`, {timeout: 5000})).status(), 403);
  assert.equal(forbiddenHits, 0);
  assert.equal((await page.goto(origin, {timeout: 5000})).status(), 200);
  await page.locator('#secure').filter({hasText: 'owned secure websocket'}).waitFor({timeout: 3000});
  assert.equal(await page.locator('#secure').innerText(), 'owned secure websocket');
  assert.equal(forbiddenHits, 0);
});

test('real Chromium permits the owned WebSocket and blocks a foreign upgrade', {
  skip: !process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH,
}, async t => {
  const playwright = require(process.env.DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH);
  let forbiddenHits = 0;
  const forbidden = http.createServer((_req, res) => {forbiddenHits++; res.end();});
  forbidden.on('upgrade', (_req, socket) => {forbiddenHits++; socket.destroy();});
  const destination = await listen(forbidden);
  t.after(() => {forbidden.closeAllConnections(); return new Promise(resolve => forbidden.close(resolve));});
  const peers = new Set();
  let origin;
  const source = http.createServer((_req, res) => {
    res.end(`<html><body><p id="owned">pending</p><p id="blocked">pending</p><script>
      const owned = new WebSocket('${origin.replace('http:', 'ws:')}/socket');
      owned.onmessage = event => document.querySelector('#owned').textContent = event.data;
      const blocked = new WebSocket('${destination.replace('http:', 'ws:')}/socket');
      blocked.onerror = () => document.querySelector('#blocked').textContent = 'denied';
    </script></body></html>`);
  });
  source.on('upgrade', (req, socket) => {
    peers.add(socket); socket.once('close', () => peers.delete(socket)); socket.on('error', () => {});
    const accept = createHash('sha1').update(req.headers['sec-websocket-key'] +
      '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const text = Buffer.from('owned websocket');
    socket.write(Buffer.concat([Buffer.from([0x81, text.length]), text]));
  });
  origin = await listen(source);
  t.after(() => {
    for (const socket of peers) socket.destroy();
    source.closeAllConnections(); return new Promise(resolve => source.close(resolve));
  });
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const browser = await playwright.chromium.launch({headless: true, timeout: 15000});
  t.after(() => browser.close());
  const context = await browser.newContext({serviceWorkers: 'block',
    proxy: {server: proxy.serverUrl, bypass: '<-loopback>'}});
  t.after(() => context.close());
  const page = await context.newPage();
  page.on('console', message => {if (message.type() === 'error') t.diagnostic(message.text().slice(0, 1024));});
  await page.goto(origin, {timeout: 5000});
  await page.locator('#owned').filter({hasText: 'owned websocket'}).waitFor({timeout: 3000});
  await page.locator('#blocked').filter({hasText: 'denied'}).waitFor({timeout: 3000});
  assert.equal(await page.locator('#owned').innerText(), 'owned websocket');
  assert.equal(await page.locator('#blocked').innerText(), 'denied');
  assert.equal(forbiddenHits, 0);
  assert.ok(proxy.metrics().denied >= 1);
});
