import assert from "node:assert/strict";
import { test } from "node:test";
import { CodemodeSandbox } from "@earendil-works/pi-codemode";

const cap = 1048576;
for (const expression of ["{number:NaN}", "{number:Infinity}", "{number:9007199254740992}",
  "{value:undefined}", "{value:()=>1}", "{value:Symbol('x')}", "new Date()",
  "({ get value() { return 1; } })", "{value:[,1]}"]) {
  test(`unsupported JSON arguments cannot execute a capability: ${expression}`, async () => {
    let effects = 0;
    const result = await run(`try { await tools.fixture(${expression}); } catch {} return true;`,
      [{ name: "fixture", execute: () => { effects++; return null; } }]);
    assert.equal(effects, 0);
    assert.equal(result.ok, true);
  });
}
async function run(code, tools = [], options = {}) {
  const sandbox = new CodemodeSandbox({
    workerUrl: new URL("./bounded-worker.mjs", import.meta.url),
    timeoutMs: 2000, memoryLimitBytes: 64 * 1024 * 1024, tools,
    ...options,
  });
  try { return await sandbox.execute(code, { store: {} }); }
  finally { await sandbox.close(); }
}
const bytes = (r) => r.output.reduce((n, x) => n + Buffer.byteLength(x.text), 0)
  + (r.ok && r.value !== undefined ? Buffer.byteLength(JSON.stringify(r.value)) : 0);

test("argument serialization uses validated snapshots instead of proxy getters", async () => {
  let received;
  const result = await run(`const args = new Proxy({number:1}, {
    get(target, key) { return key === 'number' ? NaN : target[key]; }
  }); return await tools.fixture(args);`, [
    {name:"fixture", execute(args) { received = args; return args; }},
  ]);
  assert.equal(result.ok, true);
  assert.deepEqual(received, {number:1});
});

test("guest prototype serializers cannot modify validated tool arguments", async () => {
  let received;
  const result = await run(`Array.prototype.toJSON = () => [NaN];
    Object.prototype.toJSON = () => ({number:NaN});
    return await tools.fixture({items:[1,2]});`, [
    {name:"fixture", execute(args) { received = args; return null; }},
  ]);
  assert.equal(result.ok, true);
  assert.deepEqual(received, {items:[1,2]});
});

test("bounded worker computes and dispatches an injected fake", async () => {
  const r = await run("return await tools.fixture({id:'A'});", [
    { name: "fixture", execute: (args) => args },
  ]);
  assert.equal(r.ok, true);
  assert.deepEqual(r.value, { id: "A" });
});

test("adapted guest has no Node, network, module or host file access", async () => {
  const result = await run("return ['process','require','fetch','WebAssembly'].map(name => typeof globalThis[name]);");
  assert.equal(result.ok, true);
  assert.deepEqual(result.value, ["undefined","undefined","undefined","undefined"]);
  assert.equal((await run("return await import('node:fs');")).ok, false);
  assert.equal((await run("return eval('typeof process');")).value, "undefined");
});

for (const code of ["while(true) {}", "while(true) await null;", "await new Promise(() => {});"]) {
  test(`adapted worker terminates stalled execution: ${code}`, async () => {
    const started = performance.now();
    const result = await run(code, [], {timeoutMs:100});
    assert.equal(result.ok, false);
    assert.ok(performance.now() - started < 10000);
  });
}

test("adapted worker handles memory and recursive stack exhaustion", async () => {
  const memory = await run("const values=[]; while(true) values.push('x'.repeat(1024*1024));", [],
    {memoryLimitBytes:8*1024*1024});
  assert.equal(memory.ok, false);
  assert.equal((await run("function recurse(){ return recurse(); } recurse();")).ok, false);
});

test("scratch data does not survive a fresh adapted VM", async () => {
  assert.equal((await run("store('previous',true); return load('previous');")).value, true);
  assert.equal((await run("return load('previous');")).value, undefined);
});

for (const [label, code] of [
  ["single oversized text", "text('x'.repeat(2097152));"],
  ["many text items", "for(let i=0;i<2048;i++) text('x'.repeat(1024));"],
  ["UTF8 multibyte console", "console.log('界'.repeat(400000));"],
  ["serialized return", "return 'x'.repeat(2097152);"],
  ["combined output and return", "text('x'.repeat(800000)); return 'y'.repeat(400000);"],
  ["caught output failure is terminal", "try{text('x'.repeat(2097152));}catch{} return true;"],
]) {
  test(`collector rejects ${label} before retaining more than one MiB`, async () => {
    const r = await run(code);
    assert.equal(r.ok, false);
    assert.match(r.error.message, /LIMIT_EXCEEDED/);
    assert.ok(bytes(r) <= cap);
  });
}

test("catching output exhaustion cannot dispatch another capability", async () => {
  let effects = 0;
  const r = await run("try{text('x'.repeat(2097152));}catch{} await tools.effect({});", [
    { name: "effect", execute: () => { effects++; } },
  ]);
  assert.equal(r.ok, false);
  assert.equal(effects, 0);
});

test("exact byte boundary includes JSON return quotes", async () => {
  const r = await run("return '界'.repeat(349524) + 'xx';");
  assert.equal(r.ok, true);
  assert.equal(bytes(r), cap);
});

test("image output is explicitly unsupported and never retained", async () => {
  const r = await run("image('data:image/png;base64,iVBORw0KGgo=');");
  assert.equal(r.ok, false);
  assert.match(r.error.message, /unsupported image/i);
  assert.equal(r.output.length, 0);
});

test("oversized arguments fail before fake effects", async () => {
  let effects = 0;
  const r = await run("await tools.effect({s:'x'.repeat(65536)});", [
    { name: "effect", execute: () => { effects++; } },
  ]);
  assert.equal(r.ok, false);
  assert.match(r.error.message, /LIMIT_EXCEEDED/);
  assert.equal(effects, 0);
});
