'use strict';

const http = require('node:http');
const net = require('node:net');

// This boundary is owned by one browser context. It never follows redirects:
// every subsequent absolute request must pass the same origin check.
async function createOriginProxy(origins) {
  if (!Array.isArray(origins) || origins.length === 0 || origins.length > 32) {
    throw new Error('A bounded trusted origin list is required');
  }
  const allowed = new Set(origins.map(value => {
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password ||
        value !== url.origin) {
      throw new Error('Trusted origins must be canonical HTTP(S) origins');
    }
    return url.origin;
  }));
  const sockets = new Set();
  const outbound = new Set();
  const tunnelSockets = new Set();
  const httpTunnelOrigins = new WeakMap();
  const reservedTunnels = new WeakSet();
  let denied = 0;
  let activeRequests = 0;
  let totalRequests = 0;
  let activeTunnels = 0;
  let closing;

  function reject(response, status = 403) {
    denied++;
    response.writeHead(status, {'connection': 'close', 'content-type': 'text/plain'});
    response.end('Browser network request denied');
  }

  const server = http.createServer({maxHeaderSize: 16 * 1024}, (request, response) => {
    let target;
    try {
      if (request.url.length > 8192) throw new Error('Oversized URL');
      target = new URL(request.url);
      if (closing || httpTunnelOrigins.has(request.socket) || target.protocol !== 'http:' || target.username || target.password ||
          !allowed.has(target.origin)) throw new Error('Unauthorized target');
    } catch {
      reject(response);
      return;
    }
    if (activeRequests + activeTunnels >= 32 || totalRequests >= 4096) {
      reject(response, 429);
      return;
    }
    activeRequests++;
    totalRequests++;
    let finished = false;
    function release() {
      if (!finished) { finished = true; activeRequests--; }
    }
    response.once('close', release);
    const headers = {...request.headers, host: target.host, connection: 'close'};
    const connectionHeaders = String(request.headers.connection || '').split(',');
    for (const name of [...connectionHeaders, 'proxy-authorization', 'proxy-connection',
      'keep-alive', 'transfer-encoding', 'upgrade', 'te', 'trailer']) {
      delete headers[name.trim().toLowerCase()];
    }
    const upstream = http.request(target, {method: request.method, headers, agent: false}, incoming => {
      const replyHeaders = {...incoming.headers, connection: 'close'};
      for (const name of String(incoming.headers.connection || '').split(',')) {
        delete replyHeaders[name.trim().toLowerCase()];
      }
      delete replyHeaders['transfer-encoding'];
      delete replyHeaders['keep-alive'];
      response.writeHead(incoming.statusCode, replyHeaders);
      let received = 0;
      incoming.on('data', bytes => {
        received += bytes.length;
        if (received > 16 * 1024 * 1024) {
          incoming.destroy(); response.destroy(); upstream.destroy();
        }
      });
      incoming.on('error', () => response.destroy());
      incoming.pipe(response);
    });
    outbound.add(upstream);
    upstream.once('close', () => outbound.delete(upstream));
    const deadline = setTimeout(() => upstream.destroy(new Error('Browser network deadline')), 15000);
    deadline.unref();
    upstream.once('close', () => clearTimeout(deadline));
    upstream.on('error', () => {
      if (!response.headersSent && !response.destroyed) {
        response.writeHead(502, {connection: 'close'});
        response.end('Browser upstream unavailable');
      } else response.destroy();
    });
    request.on('aborted', () => upstream.destroy());
    response.once('close', () => upstream.destroy());
    let sent = 0;
    request.on('data', bytes => {
      sent += bytes.length;
      if (sent > 1024 * 1024) {
        upstream.destroy(); request.destroy(); response.destroy();
      }
    });
    request.pipe(upstream);
  });
  server.headersTimeout = 5000;
  server.requestTimeout = 15000;
  server.maxRequestsPerSocket = 128;
  server.on('connection', socket => {
    if (sockets.has(socket)) return;
    if (closing || sockets.size >= 64) { socket.destroy(); return; }
    sockets.add(socket);
    socket.setTimeout(15000, () => socket.destroy());
    socket.once('close', () => sockets.delete(socket));
    socket.on('error', () => {});
  });
  function denyTunnel(socket, status = 403) {
    denied++;
    socket.end(`HTTP/1.1 ${status} Denied\r\nConnection: close\r\nContent-Length: 0\r\n\r\n`);
  }
  function reserveTunnel(socket) {
    if (!closing && reservedTunnels.has(socket)) return true;
    if (closing || activeRequests + activeTunnels >= 32 || totalRequests >= 4096) {
      denyTunnel(socket, 429);
      return false;
    }
    activeTunnels++; totalRequests++;
    reservedTunnels.add(socket);
    socket.once('close', () => {activeTunnels--;});
    return true;
  }
  function trackTunnel(socket) {
    if (tunnelSockets.has(socket)) return;
    tunnelSockets.add(socket);
    socket.once('close', () => tunnelSockets.delete(socket));
    socket.on('error', () => {});
  }
  function joinTunnel(client, remote, clientHead = Buffer.alloc(0), remoteHead = Buffer.alloc(0)) {
    trackTunnel(remote);
    let bytes = clientHead.length + remoteHead.length;
    const stop = () => {client.destroy(); remote.destroy();};
    const deadline = setTimeout(stop, 15000);
    deadline.unref();
    client.once('close', () => {clearTimeout(deadline); remote.destroy();});
    remote.once('close', () => {clearTimeout(deadline); client.destroy();});
    function count(chunk) {bytes += chunk.length; if (bytes > 16 * 1024 * 1024) stop();}
    client.on('data', count); remote.on('data', count);
    if (bytes > 16 * 1024 * 1024) {stop(); return;}
    if (clientHead.length) remote.write(clientHead);
    if (remoteHead.length) client.write(remoteHead);
    client.pipe(remote); remote.pipe(client);
  }
  server.on('connect', (request, client, head) => {
    let target;
    let httpOrigin;
    try {
      target = new URL(`https://${request.url}`);
      httpOrigin = new URL(`http://${request.url}`).origin;
      // CONNECT permits only an exact authority, never a URL, path or userinfo.
      if (closing || httpTunnelOrigins.has(client) || request.url.length > 8192 ||
          request.url !== `${target.hostname}:${target.port || 443}` || target.username || target.password ||
          !(allowed.has(target.origin) || allowed.has(httpOrigin))) {
        throw new Error('Unauthorized CONNECT');
      }
    } catch {denyTunnel(client); return;}
    if (!reserveTunnel(client)) return;
    if (!allowed.has(target.origin)) {
      // Chromium tunnels ws:// too. Reuse Node's HTTP parser inside that
      // tunnel, accepting only a matching WebSocket upgrade before outbound I/O.
      httpTunnelOrigins.set(client, httpOrigin);
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      server.emit('connection', client);
      if (head.length) client.unshift(head);
      return;
    }
    const remote = net.connect({host: target.hostname.replace(/^\[|\]$/g, ''), port: target.port || 443});
    trackTunnel(remote);
    const deadline = setTimeout(() => {client.destroy(); remote.destroy();}, 15000);
    deadline.unref();
    client.once('close', () => {clearTimeout(deadline); remote.destroy();});
    remote.once('error', () => {clearTimeout(deadline); denyTunnel(client, 502);});
    remote.once('connect', () => {
      clearTimeout(deadline);
      if (closing || client.destroyed) {remote.destroy(); return;}
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      joinTunnel(client, remote, head);
    });
  });
  server.on('upgrade', (request, client, head) => {
    let target;
    try {
      const tunnelOrigin = httpTunnelOrigins.get(client);
      if (tunnelOrigin && request.headers.host !== new URL(tunnelOrigin).host) {
        throw new Error('Mismatched tunneled Host');
      }
      target = tunnelOrigin ? new URL(request.url, tunnelOrigin) : new URL(request.url);
      if (tunnelOrigin && target.origin !== tunnelOrigin) throw new Error('Mismatched tunneled target');
      if (target.protocol === 'ws:') target.protocol = 'http:';
      if (closing || request.url.length > 8192 || target.protocol !== 'http:' ||
          target.username || target.password || !allowed.has(target.origin) ||
          request.method !== 'GET' || String(request.headers.upgrade).toLowerCase() !== 'websocket') {
        throw new Error('Unauthorized upgrade');
      }
    } catch {denyTunnel(client); return;}
    if (!reserveTunnel(client)) return;
    const headers = {...request.headers};
    for (const name of [...String(request.headers.connection || '').split(','),
      'proxy-authorization', 'proxy-connection', 'keep-alive', 'transfer-encoding', 'te', 'trailer']) {
      delete headers[name.trim().toLowerCase()];
    }
    Object.assign(headers, {host: target.host, connection: 'Upgrade', upgrade: 'websocket'});
    const upstream = http.request(target, {method: 'GET', headers, agent: false, maxHeaderSize: 16 * 1024});
    outbound.add(upstream);
    const deadline = setTimeout(() => {upstream.destroy(); client.destroy();}, 15000);
    deadline.unref();
    upstream.once('close', () => {clearTimeout(deadline); outbound.delete(upstream);});
    client.once('close', () => upstream.destroy());
    upstream.on('error', () => denyTunnel(client, 502));
    upstream.on('response', response => {response.destroy(); denyTunnel(client, 502);});
    upstream.once('upgrade', (response, remote, remoteHead) => {
      clearTimeout(deadline);
      if (closing || client.destroyed || response.statusCode !== 101 ||
          String(response.headers.upgrade).toLowerCase() !== 'websocket') {
        remote.destroy(); denyTunnel(client, 502); return;
      }
      const reply = Object.entries(response.headers).map(([name, value]) =>
        (Array.isArray(value) ? value : [value]).map(item => `${name}: ${item}\r\n`).join('')).join('');
      client.write(`HTTP/1.1 101 Switching Protocols\r\n${reply}\r\n`);
      joinTunnel(client, remote, head, remoteHead);
    });
    upstream.end();
  });
  server.on('clientError', (_error, socket) => socket.destroy());
  await new Promise((resolve, rejectListen) => {
    server.once('error', rejectListen);
    server.listen(0, '127.0.0.1', () => {
      server.removeListener('error', rejectListen);
      resolve();
    });
  });
  return {
    serverUrl: `http://127.0.0.1:${server.address().port}`,
    metrics: () => ({denied, activeSockets: sockets.size, activeRequests, activeTunnels, totalRequests}),
    close() {
      if (!closing) {
        const ownedSockets = new Set([...sockets, ...tunnelSockets]);
        const closedSockets = [...ownedSockets].map(socket =>
          new Promise(resolve => socket.once('close', resolve)));
        const closedServer = new Promise(resolve => server.close(resolve));
        closing = Promise.all([closedServer, ...closedSockets]).then(() => {});
        for (const request of outbound) request.destroy();
        for (const socket of ownedSockets) socket.destroy();
      }
      return closing;
    },
  };
}

module.exports = {createOriginProxy};
