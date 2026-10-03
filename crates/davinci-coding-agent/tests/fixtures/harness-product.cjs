// Synthetic, loopback-only acceptance app. No external service or credentials.
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const [directory, mode, instance] = process.argv.slice(2);
const data = path.join(directory, 'data.json');
if (mode === 'startup-failure') {
  console.error('STARTUP_FAILED synthetic configuration');
  process.exit(12);
}
if (!fs.existsSync(data)) fs.writeFileSync(data, JSON.stringify({ version: 1, value: 'empty' }));
const server = http.createServer((req, res) => {
  const reply = (status, body) => { res.writeHead(status, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(body)); };
  if (req.url === '/health') return reply(fs.existsSync(path.join(directory, 'outage')) ? 503 : 200, { instance });
  if (req.headers.authorization !== 'Bearer synthetic-user') return reply(401, { error: 'unauthorized' });
  if (fs.existsSync(path.join(directory, 'outage'))) { console.error('DATABASE_UNAVAILABLE'); return reply(503, { error: 'database unavailable' }); }
  if (req.url === '/hang') return; // Client timeout and owned-instance cancellation.
  if (req.url === '/migrate-fail') {
    const staged = data + '.staged';
    fs.writeFileSync(staged, JSON.stringify({ version: 2, value: 'partial' }));
    fs.unlinkSync(staged); // Transaction never replaces acknowledged data.
    console.error('MIGRATION_ROLLED_BACK');
    return reply(500, { error: 'migration rolled back' });
  }
  if (req.url === '/value' && req.method === 'POST') {
    fs.writeFileSync(data + '.staged', JSON.stringify({ version: 1, value: 'saved' }));
    fs.renameSync(data + '.staged', data);
    return reply(201, { saved: true });
  }
  if (req.url === '/value') return reply(200, JSON.parse(fs.readFileSync(data, 'utf8')));
  reply(404, { error: 'not found' });
});
server.listen(0, '127.0.0.1', () => {
  const record = { port: server.address().port, instance, pid: process.pid };
  fs.writeFileSync(path.join(directory, `${instance}.json`), JSON.stringify(record));
  console.log('READY ' + JSON.stringify(record));
});
setTimeout(() => process.exit(91), 30000);
