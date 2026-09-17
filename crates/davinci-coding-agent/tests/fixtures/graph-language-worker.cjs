// Real subprocess using the graph's existing authenticated coordinator channel.
const net = require('net');
const [host, port] = process.env.DAVINCI_TASK_COORDINATOR_ADDR.split(':');
function request(tool, args) {
  return new Promise((resolve, reject) => {
    const socket = net.connect({ host, port: Number(port) });
    const body = Buffer.from(JSON.stringify({ version: 1,
      credential: process.env.DAVINCI_TASK_COORDINATOR_CREDENTIAL, tool, args }));
    socket.setTimeout(10000, () => { socket.destroy(); reject(new Error('timeout')); });
    socket.on('error', reject);
    socket.on('connect', () => {
      const header = Buffer.alloc(4); header.writeUInt32BE(body.length);
      socket.write(Buffer.concat([header, body]));
    });
    let input = Buffer.alloc(0);
    socket.on('data', chunk => {
      input = Buffer.concat([input, chunk]);
      if (input.length < 4 || input.length < 4 + input.readUInt32BE(0)) return;
      const result = JSON.parse(input.subarray(4, 4 + input.readUInt32BE(0))).result;
      socket.end();
      if (!result.Ok || result.Ok.is_error) reject(new Error('request failed'));
      else resolve(result.Ok);
    });
  });
}
(async () => {
  await request('lsp_hover', { path: 'a.ts', line: 1, column: 1 });
  const references = await request('lsp_references', { path: 'a.ts', line: 1, column: 1, limit: 1 });
  const full = await request('retrieve_output', { id: references.details.fullResult.id });
  if (!full.content.includes('"total": 3')) throw new Error('missing retained records');
  process.stdout.write(JSON.stringify({ success: true, rss: process.memoryUsage().rss }));
})().catch(() => { process.exitCode = 1; });
