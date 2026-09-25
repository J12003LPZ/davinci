// Deterministic offline LSP fixture. Protocol stdout contains frames only.
const fs = require('fs');
const mode = process.argv[2] || 'normal';
const eventsPath = process.argv[3] || process.env.DAVINCI_LSP_EVENT_LOG || '';
let input = Buffer.alloc(0);
let initialized = false;
let pendingInitialize = null;
let delayedQuery = null;
let opens = 0;
let changes = 0;
let sequence = 0;
const texts = {};
const range = { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } };

function record(kind, message, extra = {}) {
  if (!eventsPath) return;
  const row = {
    seq: ++sequence,
    pid: process.pid,
    kind,
    method: message && message.method,
    id: message && message.id,
    ...extra,
  };
  fs.appendFileSync(eventsPath, JSON.stringify(row) + '\n');
}
function send(message) {
  record(message.method ? (message.id === undefined ? 'notification' : 'server_request') : 'response', message);
  const body = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...message }), 'utf8');
  process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`);
  process.stdout.write(body);
}
function capabilities() {
  const pushOnly = mode.startsWith('push-') || mode === 'dynamic-diagnostics' || mode === 'stale-after-current' || mode === 'slow-two-phase';
  return {
    positionEncoding: mode === 'utf8' ? 'utf-8' : 'utf-16',
    textDocumentSync: { openClose: true, change: 1, save: { includeText: true } },
    ...(mode === 'no-capabilities' ? {} : {
      hoverProvider: true,
      definitionProvider: true,
      referencesProvider: true,
      implementationProvider: true,
      typeDefinitionProvider: true,
      documentSymbolProvider: true,
      workspaceSymbolProvider: true,
      ...(pushOnly ? {} : { diagnosticProvider: { interFileDependencies: true, workspaceDiagnostics: false } }),
    }),
  };
}
function initializeResult(id) {
  send({ id, result: { capabilities: capabilities() } });
}
function publish(uri, version, diagnostics) {
  send({ method: 'textDocument/publishDiagnostics', params: { uri, version, diagnostics } });
}
function diagnosticItems(uri) {
  return (texts[uri] || '').trim() === 'bad'
    ? [{ range, severity: 1, message: 'bad type' }]
    : [];
}
function normalResult(method, message) {
  if (method === 'textDocument/hover') {
    return initialized ? { contents: 'fixture hover', fixture: { opens, changes, texts } } : null;
  }
  if (method === 'textDocument/definition' || method === 'textDocument/implementation' || method === 'textDocument/typeDefinition') {
    return [{ uri: message.params.textDocument.uri, range }];
  }
  if (method === 'textDocument/references') {
    return [0, 1, 2].map(character => ({
      uri: message.params.textDocument.uri,
      range: { start: { line: 0, character }, end: { line: 0, character: character + 1 } },
    }));
  }
  if (method === 'textDocument/documentSymbol') {
    return [{ name: 'fixture', kind: 12, range, selectionRange: range }];
  }
  if (method === 'workspace/symbol') return [];
  return undefined;
}
function handleResponse(message) {
  record('client_response', message, { result: message.result, error: message.error });
  if (message.id === 'config-before-init' && pendingInitialize !== null) {
    const id = pendingInitialize;
    pendingInitialize = null;
    initializeResult(id);
  }
}
function handle(message) {
  record('client_message', message, {
    documentVersion: message.params && message.params.textDocument && message.params.textDocument.version,
    languageId: message.params && message.params.textDocument && message.params.textDocument.languageId,
    rootUri: message.params && message.params.rootUri,
  });
  if (!message.method) return handleResponse(message);
  if (message.method === 'exit') process.exit(0);
  if (mode === 'exit') process.exit(7);
  if (mode === 'malformed') {
    process.stdout.write('Content-Length: 2\r\n\r\nxx');
    return;
  }
  if (mode === 'timeout') return;
  if (message.method === 'shutdown') {
    send({ id: message.id, result: null });
    return;
  }
  if (message.method === 'initialize') {
    if (mode === 'config-before-initialize') {
      pendingInitialize = message.id;
      send({
        id: 'config-before-init',
        method: 'workspace/configuration',
        params: { items: [
          { scopeUri: message.params.rootUri, section: 'python' },
          { scopeUri: message.params.rootUri, section: 'basedpyright' },
          { section: 'unrecognized.section' },
        ] },
      });
    } else {
      initializeResult(message.id);
    }
    return;
  }
  if (message.method === 'initialized') {
    initialized = true;
    if (mode === 'dynamic-diagnostics') {
      send({
        id: 'register-diagnostics',
        method: 'client/registerCapability',
        params: { registrations: [{
          id: 'fixture-document-diagnostics',
          method: 'textDocument/diagnostic',
          registerOptions: { documentSelector: [{ scheme: 'file' }] },
        }] },
      });
    }
    return;
  }
  if (message.method === 'textDocument/didOpen' || message.method === 'textDocument/didChange') {
    const doc = message.params.textDocument;
    if (message.method.endsWith('didOpen')) {
      opens++;
      texts[doc.uri] = message.params.textDocument.text;
    } else {
      changes++;
      texts[doc.uri] = message.params.contentChanges[0].text;
    }
    if (mode === 'blocked-reader' && message.method.endsWith('didChange')) {
      process.stdin.pause();
      setTimeout(() => process.stdin.resume(), 250);
    }
    if (mode === 'push-two-phase' || mode === 'slow-two-phase') {
      publish(doc.uri, doc.version, []);
      setTimeout(() => publish(doc.uri, doc.version, diagnosticItems(doc.uri)), mode === 'slow-two-phase' ? 300 : 120);
      return;
    }
    if (mode === 'stale-after-current') {
      publish(doc.uri, doc.version, diagnosticItems(doc.uri));
      setTimeout(() => publish(doc.uri, Math.max(0, doc.version - 1), []), 30);
      return;
    }
    if (mode === 'push-stale-diagnostics') {
      publish(doc.uri, Math.max(0, doc.version - 1), [{ range, severity: 1, message: 'stale error' }]);
    }
    if (mode.startsWith('push-')) {
      setTimeout(() => publish(doc.uri, doc.version, diagnosticItems(doc.uri)), 20);
    }
    return;
  }
  if (message.method === 'textDocument/didClose') {
    delete texts[message.params.textDocument.uri];
    return;
  }
  if (message.method === 'textDocument/diagnostic') {
    send({ id: message.id, result: { kind: 'full', resultId: `r-${sequence}`, items: diagnosticItems(message.params.textDocument.uri) } });
    return;
  }
  if (mode === 'late-once' && /^textDocument\//.test(message.method) && message.id !== undefined) {
    if (!delayedQuery) {
      delayedQuery = message;
      return;
    }
    const result = normalResult(message.method, message);
    send({ id: message.id, result: result === undefined ? null : result });
    const first = delayedQuery;
    delayedQuery = null;
    setTimeout(() => {
      const delayed = normalResult(first.method, first);
      send({ id: first.id, result: delayed === undefined ? null : delayed });
    }, 20);
    return;
  }
  const result = normalResult(message.method, message);
  if (result !== undefined) {
    send({ id: message.id, result });
    return;
  }
  if (message.method === 'fixture/echo') {
    process.stderr.write('x'.repeat(65536));
    publish(message.params.uri, 1, [{ message: 'fixture diagnostic', severity: 1, range }]);
    send({ id: 'edit-probe', method: 'workspace/applyEdit', params: { edit: {} } });
    setTimeout(() => send({ id: message.id, result: message.params }), message.params.delay || 0);
    return;
  }
  if (message.method === 'fixture/configuration') {
    send({
      id: 'fixture-config',
      method: 'workspace/configuration',
      params: { items: message.params.items || [] },
    });
    send({ id: message.id, result: true });
    return;
  }
  if (message.method === 'fixture/refresh') {
    send({ id: 'fixture-refresh', method: 'workspace/diagnostic/refresh', params: {} });
    send({ id: message.id, result: true });
    return;
  }
  if (message.method === 'fixture/orphan') {
    const child = require('child_process').spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
      stdio: 'ignore',
      windowsHide: true,
    });
    child.unref();
    send({ id: message.id, result: child.pid });
    setTimeout(() => process.exit(0), 20);
    return;
  }
  if (message.id !== undefined) {
    send({ id: message.id, error: { code: -32601, message: 'fixture unsupported method' } });
  }
}
process.stdin.on('data', chunk => {
  input = Buffer.concat([input, chunk]);
  for (;;) {
    const end = input.indexOf('\r\n\r\n');
    if (end < 0) break;
    const match = /Content-Length: (\d+)/i.exec(input.subarray(0, end).toString('ascii'));
    if (!match) process.exit(9);
    const size = Number(match[1]);
    if (input.length < end + 4 + size) break;
    const message = JSON.parse(input.subarray(end + 4, end + 4 + size).toString('utf8'));
    input = input.subarray(end + 4 + size);
    handle(message);
  }
});
record('spawn', { method: 'spawn' }, { mode });
