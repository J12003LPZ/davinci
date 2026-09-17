'use strict';

const http = require('node:http');

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
  let denied = 0;
  let activeRequests = 0;
  let totalRequests = 0;
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
      if (closing || target.protocol !== 'http:' || target.username || target.password ||
          !allowed.has(target.origin)) throw new Error('Unauthorized target');
    } catch {
      reject(response);
      return;
    }
    if (activeRequests >= 32 || totalRequests >= 4096) {
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
    if (closing || sockets.size >= 64) { socket.destroy(); return; }
    sockets.add(socket);
    socket.setTimeout(15000, () => socket.destroy());
    socket.once('close', () => sockets.delete(socket));
    socket.on('error', () => {});
  });
  function denyTunnel(_request, socket) {
    denied++;
    socket.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n');
  }
  // Authorized HTTPS and WebSocket support is added with its own acceptance
  // tests; unsupported tunnel forms must never escape the boundary meanwhile.
  server.on('connect', denyTunnel);
  server.on('upgrade', denyTunnel);
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
    metrics: () => ({denied, activeSockets: sockets.size, activeRequests, totalRequests}),
    close() {
      if (!closing) {
        closing = new Promise(resolve => {
          server.close(resolve);
          for (const request of outbound) request.destroy();
          for (const socket of sockets) socket.destroy();
        }).then(() => { sockets.clear(); });
      }
      return closing;
    },
  };
}

module.exports = {createOriginProxy};
