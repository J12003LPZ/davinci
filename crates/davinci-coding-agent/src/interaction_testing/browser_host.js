'use strict';

// This entry point is materialized from embedded host bytes, outside the project.
// Its configuration is written by the Rust owner, never by a model tool call.
const fs = require('node:fs');
const path = require('node:path');
const {createHash, randomBytes} = require('node:crypto');
const {loadTrustedPlaywright, createBrowserBackend} = require('./browser_backend.js');
const {createBrowserTransport} = require('./browser_transport.js');

function createArtifactSink(directory) {
  const staged = new Map();
  return async (_resource, {bytes, mediaType}) => {
    if (!Buffer.isBuffer(bytes) || bytes.length > 4 * 1024 * 1024 || mediaType !== 'image/png') {
      throw new Error('Invalid screenshot transfer');
    }
    for (const hash of staged.keys()) {
      if (!fs.existsSync(path.join(directory, hash + '.png'))) staged.delete(hash);
    }
    if (staged.size >= 8) throw new Error('Screenshot transfer admission limit');
    const hash = createHash('sha256').update(bytes).digest('hex');
    const transfer = randomBytes(32).toString('hex');
    // Separate transfer identities prevent one context consuming another's file.
    const file = path.join(directory, transfer + '.png');
    fs.writeFileSync(file, bytes, {flag: 'wx', mode: 0o600});
    staged.set(transfer, bytes.length);
    return {artifact: 'staging:' + transfer + ':' + hash, size: bytes.length, mediaType};
  };
}

async function main() {
  if (process.argv.length !== 3) throw new Error('Invalid browser host arguments');
  // macOS exposes the temporary directory through a symlink (`/var` to
  // `/private/var`). Canonicalize both sides before enforcing that the host
  // can only read the Rust-owned config beside its materialized script.
  const hostDirectory = fs.realpathSync(__dirname);
  const file = fs.realpathSync(path.resolve(process.argv[2]));
  if (path.dirname(file) !== hostDirectory || fs.statSync(file).size > 16 * 1024) {
    throw new Error('Invalid browser host configuration');
  }
  const config = JSON.parse(fs.readFileSync(file, 'utf8'));
  const backend = createBrowserBackend(loadTrustedPlaywright(config));
  const transport = createBrowserTransport({input: process.stdin, output: process.stdout,
    backend, artifact: createArtifactSink(__dirname)});
  const close = () => {void transport.close();};
  process.once('SIGTERM', close);
  process.once('SIGINT', close);
  const outcome = await transport.done;
  // Paused stdin remains an active pipe after a protocol shutdown. Cleanup is
  // complete and response writes were acknowledged before closing the streams.
  process.stdin.destroy();
  process.stdout.end();
  process.exitCode = outcome.failed ? 1 : 0;
}

if (require.main === module) main().catch(error => {
  // Dependency errors can contain sensitive paths. Keep the normal transport
  // message generic; the opt-in diagnostic is limited to the native CI probe.
  if (process.env.DAVINCI_BROWSER_DIAGNOSTICS === '1') {
    const detail = error && typeof error.message === 'string' ? error.message.slice(0, 512) : 'unknown error';
    process.stderr.write(`Browser host unavailable: ${detail}\n`);
  } else {
    process.stderr.write('Browser host unavailable\n');
  }
  process.exitCode = 1;
});
module.exports = {createArtifactSink};
