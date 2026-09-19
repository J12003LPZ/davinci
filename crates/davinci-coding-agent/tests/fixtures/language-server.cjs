// Offline LSP fixture. Deliberately interleaves server requests/notifications.
const mode = process.argv[2] || 'normal';
let input = Buffer.alloc(0);
let initialized = false;
const texts = {};
let opens = 0, changes = 0;
const range = { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } };
function send(message) {
  const body = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...message }));
  process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`);
  process.stdout.write(body);
}
process.stdin.on('data', chunk => {
  input = Buffer.concat([input, chunk]);
  for (;;) {
    const end = input.indexOf('\r\n\r\n');
    if (end < 0) break;
    const size = Number(/Content-Length: (\d+)/i.exec(input.subarray(0, end).toString())[1]);
    if (input.length < end + 4 + size) break;
    const message = JSON.parse(input.subarray(end + 4, end + 4 + size));
    input = input.subarray(end + 4 + size);
    if (!message.method) {
      if (message.id === 'edit-probe') send({ method: 'fixture/edit-result', params: message.result });
      continue;
    }
    if (message.method === 'exit') process.exit(0);
    if (mode === 'exit') process.exit(7);
    if (mode === 'malformed') { process.stdout.write('Content-Length: 2\r\n\r\nxx'); continue; }
    if (mode === 'timeout') continue;
    if (message.method === 'shutdown') { send({ id: message.id, result: null }); continue; }
    if (message.method === 'initialize') {
      send({ id: message.id, result: { capabilities: {
        positionEncoding: mode === 'utf8' ? 'utf-8' : 'utf-16',
        textDocumentSync: { openClose: true, change: 1, save: { includeText: true } },
        ...(mode === 'no-capabilities' ? {} : { hoverProvider: true, definitionProvider: true,
          referencesProvider: true, implementationProvider: true, typeDefinitionProvider: true,
          documentSymbolProvider: true, workspaceSymbolProvider: true,
          ...(mode.startsWith('push-') ? {} : { diagnosticProvider: { interFileDependencies: true, workspaceDiagnostics: false } }) })
      } } }); continue;
    }
    if (message.method === 'initialized') { initialized = true; continue; }
    if (message.method === 'textDocument/didOpen' || message.method === 'textDocument/didChange') {
      const doc = message.params.textDocument;
      if (message.method.endsWith('didOpen')) { opens++; texts[doc.uri] = doc.text; }
      else { changes++; texts[doc.uri] = message.params.contentChanges[0].text; }
      if (mode === 'push-stale-diagnostics') send({ method: 'textDocument/publishDiagnostics', params: {
        uri: doc.uri, version: doc.version - 1, diagnostics: [{ range, severity: 1, message: 'stale error' }]
      } });
      if (mode.startsWith('push-')) setTimeout(() => send({ method: 'textDocument/publishDiagnostics', params: {
        uri: doc.uri, version: doc.version, diagnostics: texts[doc.uri].trim() === 'bad' ? [{ range, severity: 1, message: 'bad type' }] : []
      } }), 20);
      continue;
    }
    if (message.method === 'textDocument/didClose') { delete texts[message.params.textDocument.uri]; continue; }
    if (message.method === 'textDocument/hover') {
      send({ id: message.id, result: initialized ? { contents: 'fixture hover', fixture: { opens, changes, texts } } : null }); continue;
    }
    if (message.method === 'textDocument/diagnostic') {
      send({ id: message.id, result: { kind: 'full', items: texts[message.params.textDocument.uri].trim() === 'bad' ? [{ range, severity: 1, message: 'bad type' }] : [] } }); continue;
    }
    if (message.method === 'workspace/symbol') { send({ id: message.id, result: [] }); continue; }
    if (message.method === 'textDocument/references') {
      send({ id: message.id, result: [0, 1, 2].map(character => ({ uri: message.params.textDocument.uri,
        range: { start: { line: 0, character }, end: { line: 0, character: character + 1 } } })) });
      continue;
    }
    if (message.method === 'fixture/echo') {
      process.stderr.write('x'.repeat(65536));
      send({ method: 'textDocument/publishDiagnostics', params: {
        uri: message.params.uri, version: 1,
        diagnostics: [{ message: 'fixture diagnostic', severity: 1,
          range: { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } } }]
      } });
      send({ id: 'edit-probe', method: 'workspace/applyEdit', params: { edit: {} } });
      setTimeout(() => send({ id: message.id, result: message.params }), message.params.delay || 0);
    }
    if (message.method === 'fixture/orphan') {
      const child = require('child_process').spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore', windowsHide: true });
      child.unref();
      send({ id: message.id, result: child.pid });
      setTimeout(() => process.exit(0), 20);
    }
  }
});
