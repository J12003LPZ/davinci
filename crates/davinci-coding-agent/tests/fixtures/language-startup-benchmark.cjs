// Offline product startup probe: node language-startup-benchmark.cjs <davinci executable>
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const assert = require('node:assert/strict');

const executable = fs.realpathSync(process.argv[2]);
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'davinci-language-startup-'));
const config = path.join(root, 'config');
const server = path.join(root, 'node_modules/typescript-language-server');
fs.mkdirSync(config);
fs.mkdirSync(server, { recursive: true });
fs.writeFileSync(path.join(server, 'package.json'), JSON.stringify({
  name: 'typescript-language-server', version: 'fixture', bin: 'server.cjs',
}));
fs.writeFileSync(path.join(server, 'server.cjs'),
  "require('node:fs').writeFileSync('unexpected-server-start', 'started'); process.exit(1);");
fs.writeFileSync(path.join(root, 'a.ts'), 'export const value = 1;');

const env = { ...process.env, PI_CODING_AGENT_DIR: config, DAVINCI_CODING_AGENT_DIR: config,
  PI_OFFLINE: '1', DAVINCI_OFFLINE: '1', PI_DISABLE_NETWORK: '1', PI_HOOKS_DRY_RUN: '1' };
for (const key of Object.keys(env)) {
  if (key.startsWith('PI_GRAPH_') || key.startsWith('DAVINCI_TASK_') ||
      key.startsWith('PI_OFFLINE_TOOL_') || /TOKEN|SECRET|API_KEY/.test(key)) delete env[key];
}
const samples = { enabled: [], disabled: [] };
for (let round = 0; round < 6; round++) {
  for (const enabled of round % 2 ? [false, true] : [true, false]) {
    fs.writeFileSync(path.join(config, 'settings.json'), JSON.stringify({
      languageIntelligence: { enabled },
    }));
    const start = performance.now();
    const result = spawnSync(executable,
      ['--offline', '--no-extensions', '--no-skills', '--no-prompt-templates', '--mode', 'rpc'],
      { cwd: root, env, input: '{"type":"get_state","id":"startup"}\n',
        encoding: 'utf8', timeout: 15000, windowsHide: true });
    const elapsed = performance.now() - start;
    assert.equal(result.error, undefined);
    assert.equal(result.status, 0);
    const responses = result.stdout.trim().split('\n').map(line => JSON.parse(line));
    assert(responses.some(value => value.id === 'startup' && value.success === true));
    assert.equal(fs.existsSync(path.join(root, 'unexpected-server-start')), false);
    if (round > 0) samples[enabled ? 'enabled' : 'disabled'].push(elapsed);
  }
}
const report = Object.fromEntries(Object.entries(samples).map(([key, values]) => {
  values.sort((a, b) => a - b);
  return [key, { samples: values.length, medianMs: values[2], maxMs: values[4] }];
}));
console.log(JSON.stringify({ ...report, lspProcessesStarted: 0,
  measurement: 'offline RPC startup, get_state response and clean exit; first round excluded',
  fixture: root }, null, 2));
