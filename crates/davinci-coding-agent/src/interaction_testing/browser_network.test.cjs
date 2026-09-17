const {test} = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const net = require('node:net');
const {createOriginProxy} = require('./browser_network.js');

async function listen(server) {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return `http://127.0.0.1:${server.address().port}`;
}
function request(proxy, url, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = http.request(proxy, {path: url, headers}, res => {
      let text = '';
      res.on('data', bytes => { text += bytes; });
      res.on('end', () => resolve({status: res.statusCode, text}));
    });
    req.on('error', reject); req.end();
  });
}

test('proxy validates canonical trusted origins without starting a listener', async () => {
  for (const origins of [[], ['file:///private'], ['http://user:secret@127.0.0.1:1'], ['http://127.0.0.1:1/path'], ['http://127.0.0.1:1/?key=secret']]) {
    await assert.rejects(createOriginProxy(origins));
  }
});

test('proxy permits owned HTTP and supplies the authorized Host header', async t => {
  let receivedHost;
  const server = http.createServer((req, res) => { receivedHost = req.headers.host; res.end('owned'); });
  const origin = await listen(server);
  t.after(() => new Promise(resolve => server.close(resolve)));
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  assert.deepEqual(await request(proxy.serverUrl, `${origin}/login`, {host: 'external.example'}), {status: 200, text: 'owned'});
  assert.equal(receivedHost, new URL(origin).host);
});

test('proxy refuses another port, userinfo and relative targets before outbound I/O', async t => {
  let hits = 0;
  const server = http.createServer((_req, res) => { hits++; res.end('forbidden'); });
  const origin = await listen(server);
  t.after(() => new Promise(resolve => server.close(resolve)));
  const proxy = await createOriginProxy(['http://127.0.0.1:1']);
  t.after(() => proxy.close());
  for (const target of [`${origin}/secret`, 'http://user:secret@127.0.0.1:1/', '/relative', 'file:///private']) {
    assert.equal((await request(proxy.serverUrl, target)).status, 403);
  }
  assert.equal(hits, 0);
  assert.equal(proxy.metrics().denied, 4);
});

test('proxy does not automatically follow a redirect outside policy', async t => {
  let forbiddenHits = 0;
  const forbidden = http.createServer((_req, res) => { forbiddenHits++; res.end('forbidden'); });
  const forbiddenOrigin = await listen(forbidden);
  t.after(() => new Promise(resolve => forbidden.close(resolve)));
  const owned = http.createServer((_req, res) => { res.writeHead(302, {location: `${forbiddenOrigin}/target`}); res.end(); });
  const origin = await listen(owned);
  t.after(() => new Promise(resolve => owned.close(resolve)));
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  assert.equal((await request(proxy.serverUrl, `${origin}/redirect`)).status, 302);
  assert.equal((await request(proxy.serverUrl, `${forbiddenOrigin}/target`)).status, 403);
  assert.equal(forbiddenHits, 0);
});

test('unsupported CONNECT and upgrades fail closed without opening a tunnel', async t => {
  const proxy = await createOriginProxy(['http://127.0.0.1:1']);
  t.after(() => proxy.close());
  for (const requestLine of [
    'CONNECT 127.0.0.1:1 HTTP/1.1\r\nHost: 127.0.0.1:1\r\n\r\n',
    'GET http://127.0.0.1:1/ HTTP/1.1\r\nHost: 127.0.0.1:1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n',
  ]) {
    const response = await new Promise((resolve, reject) => {
      const socket = net.connect(new URL(proxy.serverUrl).port, '127.0.0.1');
      let data = '';
      socket.setTimeout(3000, () => socket.destroy(new Error('proxy response timeout')));
      socket.on('connect', () => socket.end(requestLine));
      socket.on('data', bytes => {data += bytes;});
      socket.on('end', () => resolve(data));
      socket.on('error', reject);
    });
    assert.match(response, /^HTTP\/1.1 403/);
  }
});

test('proxy close is idempotent and destroys held inbound connections', async () => {
  const proxy = await createOriginProxy(['http://127.0.0.1:1']);
  const socket = net.connect(new URL(proxy.serverUrl).port, '127.0.0.1');
  await new Promise((resolve, reject) => {socket.once('connect', resolve); socket.once('error', reject);});
  const closed = new Promise(resolve => socket.once('close', resolve));
  await proxy.close(); await proxy.close(); await closed;
  assert.equal(proxy.metrics().activeSockets, 0);
});

test('proxy strips proxy credentials and connection-nominated headers', async t => {
  let headers;
  const server = http.createServer((req, res) => {headers = req.headers; res.end('owned');});
  const origin = await listen(server);
  t.after(() => new Promise(resolve => server.close(resolve)));
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  assert.equal((await request(proxy.serverUrl, origin, {
    'proxy-authorization': 'fixture-only', connection: 'x-private', 'x-private': 'fixture-only',
  })).status, 200);
  assert.equal(headers['proxy-authorization'], undefined);
  assert.equal(headers['x-private'], undefined);
});

test('proxy shutdown cancels a held upstream request', async t => {
  let markStarted;
  const started = new Promise(resolve => {markStarted = resolve;});
  const server = http.createServer(() => markStarted());
  const origin = await listen(server);
  t.after(() => {server.closeAllConnections(); return new Promise(resolve => server.close(resolve));});
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const pending = request(proxy.serverUrl, origin).then(() => 'response', () => 'cancelled');
  await started;
  await proxy.close();
  assert.equal(await pending, 'cancelled');
  assert.equal(proxy.metrics().activeSockets, 0);
});

test('proxy limits simultaneous upstream requests before more outbound I/O', async t => {
  let hits = 0;
  let ready;
  const admitted = new Promise(resolve => {ready = resolve;});
  const server = http.createServer(() => {if (++hits === 32) ready();});
  const origin = await listen(server);
  t.after(() => {server.closeAllConnections(); return new Promise(resolve => server.close(resolve));});
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const pending = Array.from({length: 32}, () => request(proxy.serverUrl, origin).catch(() => null));
  await admitted;
  assert.equal((await request(proxy.serverUrl, origin)).status, 429);
  assert.equal(hits, 32);
  await proxy.close();
  await Promise.all(pending);
});

test('proxy bounds upstream response bytes and never reports a complete oversized body', async t => {
  const server = http.createServer((_req, res) => {res.end(Buffer.alloc(17 * 1024 * 1024));});
  const origin = await listen(server);
  t.after(() => {server.closeAllConnections(); return new Promise(resolve => server.close(resolve));});
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const result = await new Promise((resolve, reject) => {
    const req = http.get(proxy.serverUrl, {path: origin}, res => {
      let count = 0;
      res.on('data', bytes => {count += bytes.length;});
      res.once('aborted', () => resolve({aborted: true, count}));
      res.once('end', () => resolve({aborted: false, count}));
      res.on('error', () => {});
    });
    req.on('error', reject);
  });
  assert.equal(result.aborted, true);
  assert.ok(result.count <= 16 * 1024 * 1024);
});
