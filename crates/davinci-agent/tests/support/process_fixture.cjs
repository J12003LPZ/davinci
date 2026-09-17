// Local-only lifecycle fixture. Every process has a failsafe lifetime bound.
const fs = require('node:fs');
const net = require('node:net');
const path = require('node:path');
const { spawn } = require('node:child_process');
const readline = require('node:readline');

const [directory, label, depthText = '0', inheritedInstance] = process.argv.slice(2);
const depth = Number(depthText);
const instance = inheritedInstance || String(process.pid);
const server = net.createServer(socket => {
  socket.on('error', () => {}); // Health probes may close before reading the reply.
  socket.end(JSON.stringify({ pid: process.pid, label, depth }) + '\n');
});
server.listen(0, '127.0.0.1', () => {
  const record = { pid: process.pid, port: server.address().port, label, depth, instance };
  fs.writeFileSync(path.join(directory, `${label}.${instance}.${depth}.json`), JSON.stringify(record));
  process.stdout.write(`READY ${JSON.stringify(record)}\n`);
  if (depth > 0) {
    spawn(process.execPath, [__filename, directory, label, String(depth - 1), instance], {
      stdio: ['ignore', 'inherit', 'inherit'],
      windowsHide: true,
    });
  }
});

readline.createInterface({ input: process.stdin }).on('line', line => {
  if (line === 'burst') {
    process.stdout.write(Buffer.alloc(5 * 1024 * 1024, 'b'), () => {
      process.stdout.write('\nBURST_DONE\n');
    });
  } else {
    process.stdout.write(`ECHO ${line}\n`);
  }
});
setTimeout(() => process.exit(91), 20_000);
