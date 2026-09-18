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

test('unauthorized CONNECT and upgrades fail closed without opening a tunnel', async t => {
  const proxy = await createOriginProxy(['http://127.0.0.1:1']);
  t.after(() => proxy.close());
  for (const requestLine of [
    'CONNECT 127.0.0.1:2 HTTP/1.1\r\nHost: 127.0.0.1:2\r\n\r\n',
    'GET http://127.0.0.1:2/ HTTP/1.1\r\nHost: 127.0.0.1:2\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n',
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

test('HTTP CONNECT refuses ordinary requests and a foreign WebSocket Host before I/O', async t => {
  let hits = 0;
  const server = http.createServer((_req, res) => {hits++; res.end();});
  server.on('upgrade', (_req, socket) => {hits++; socket.destroy();});
  const origin = await listen(server);
  t.after(() => new Promise(resolve => server.close(resolve)));
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  for (const inner of [
    `GET ${origin}/private HTTP/1.1\r\nHost: ${new URL(origin).host}\r\n\r\n`,
    'GET /socket HTTP/1.1\r\nHost: foreign.example\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n',
  ]) {
    const authority = new URL(origin).host;
    const {socket, header} = await tunnel(proxy.serverUrl,
      `CONNECT ${authority} HTTP/1.1\r\nHost: ${authority}\r\n\r\n`);
    t.after(() => socket.destroy());
    assert.match(header, /^HTTP\/1.1 200/);
    const reply = new Promise(resolve => socket.once('data', bytes => resolve(bytes.toString())));
    socket.write(inner);
    assert.match(await reply, /^HTTP\/1.1 403/);
  }
  assert.equal(hits, 0);
});

test('CONNECT rejects crafted authorities before any outbound connection', async t => {
  let hits = 0;
  const peers = new Set();
  const server = net.createServer(socket => {hits++; peers.add(socket); socket.on('error', () => {});});
  const origin = (await listen(server)).replace('http:', 'https:');
  t.after(() => {for (const socket of peers) socket.destroy(); return new Promise(resolve => server.close(resolve));});
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const authority = new URL(origin).host;
  for (const target of [`user:fixture@${authority}`, `${authority}/private`, `${authority}?q=fixture`,
    `${authority}#private`, `https://${authority}`, `${authority}:443`]) {
    const result = await tunnel(proxy.serverUrl, `CONNECT ${target} HTTP/1.1\r\nHost: ${authority}\r\n\r\n`)
      .catch(error => {
        // Node's HTTP parser can reject malformed CONNECT syntax before the
        // application handler. A closed transport is also a fail-closed result.
        assert.equal(error.message, 'Tunnel closed before handshake');
        return null;
      });
    if (result) {result.socket.destroy(); assert.match(result.header, /^HTTP\/1.1 403/);}
  }
  assert.equal(hits, 0);
});

function tunnel(proxy, line) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(new URL(proxy).port, '127.0.0.1');
    let header = Buffer.alloc(0);
    socket.setTimeout(3000, () => socket.destroy(new Error('Tunnel deadline')));
    function read(bytes) {
      header = Buffer.concat([header, bytes]);
      const end = header.indexOf('\r\n\r\n');
      if (end !== -1) {
        socket.removeListener('data', read);
        resolve({socket, header: header.subarray(0, end).toString(), head: header.subarray(end + 4)});
      }
    }
    socket.on('data', read);
    socket.on('error', reject);
    socket.once('end', () => reject(new Error('Tunnel closed before handshake')));
    socket.on('connect', () => socket.write(line));
  });
}

test('authorized HTTPS CONNECT forwards bytes and shutdown destroys the tunnel', async t => {
  const peers = new Set();
  const server = net.createServer(socket => {
    peers.add(socket); socket.once('close', () => peers.delete(socket));
    socket.on('error', () => {}); socket.on('data', bytes => socket.write(bytes));
  });
  const origin = (await listen(server)).replace('http:', 'https:');
  t.after(() => {for (const peer of peers) peer.destroy(); return new Promise(resolve => server.close(resolve));});
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const authority = new URL(origin).host;
  const {socket, header} = await tunnel(proxy.serverUrl, `CONNECT ${authority} HTTP/1.1\r\nHost: ${authority}\r\n\r\n`);
  t.after(() => socket.destroy());
  assert.match(header, /^HTTP\/1.1 200/);
  const received = new Promise(resolve => socket.once('data', bytes => resolve(bytes.toString())));
  socket.write('owned tunnel');
  assert.equal(await received, 'owned tunnel');
  const closed = new Promise(resolve => socket.once('close', resolve));
  await proxy.close(); await closed;
  assert.equal(proxy.metrics().activeTunnels, 0);
});

test('authorized WebSocket upgrade preserves handshake and filters proxy credentials', async t => {
  let receivedHeaders;
  const peers = new Set();
  const server = http.createServer();
  server.on('upgrade', (req, socket, head) => {
    receivedHeaders = req.headers;
    peers.add(socket); socket.once('close', () => peers.delete(socket)); socket.on('error', () => {});
    socket.write('HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n');
    if (head.length) socket.write(head);
    socket.on('data', bytes => socket.write(bytes));
  });
  const origin = await listen(server);
  t.after(() => {for (const peer of peers) peer.destroy(); return new Promise(resolve => server.close(resolve));});
  const proxy = await createOriginProxy([origin]);
  t.after(() => proxy.close());
  const {socket, header} = await tunnel(proxy.serverUrl,
    `GET ${origin}/socket HTTP/1.1\r\nHost: wrong.example\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nProxy-Authorization: fixture-only\r\n\r\n`);
  t.after(() => socket.destroy());
  assert.match(header, /^HTTP\/1.1 101/);
  assert.equal(receivedHeaders.host, new URL(origin).host);
  assert.equal(receivedHeaders['proxy-authorization'], undefined);
  const received = new Promise(resolve => socket.once('data', bytes => resolve(bytes.toString())));
  socket.write('owned websocket fixture');
  assert.equal(await received, 'owned websocket fixture');
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
