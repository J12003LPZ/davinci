import assert from "node:assert/strict";
import { test } from "node:test";
import { CodemodeSandbox } from "@earendil-works/pi-codemode";

async function execute(code, options = {}) {
  const sandbox = new CodemodeSandbox({
    timeoutMs: 1000,
    memoryLimitBytes: 64 * 1024 * 1024,
    ...options,
  });
  try {
    return await sandbox.execute(code, { store: {} });
  } finally {
    await sandbox.close();
  }
}

test("published package returns a constant from a fresh VM", async () => {
  const result = await execute("return 42");
  assert.equal(result.ok, true);
  assert.equal(result.value, 42);
});

test("injected fake tool and caught error stay inside fixture authority", async () => {
  let calls = 0;
  const result = await execute(
    "const first = await tools.fixture({ id: 'A' }); try { await tools.fails({}); } catch {} return first;",
    { tools: [
      { name: "fixture", execute: (args) => { calls++; return { id: args.id }; } },
      { name: "fails", execute: () => { throw new Error("fixture error"); } },
    ] },
  );
  assert.equal(result.ok, true);
  assert.deepEqual(result.value, { id: "A" });
  assert.equal(calls, 1);
  assert.deepEqual(result.calls.map((call) => call.status), ["ok", "error"]);
});

test("guest cannot use Node, network, modules, or inherited environment", async () => {
  const result = await execute(
    "return ['process', 'require', 'fetch', 'WebAssembly'].map(name => typeof globalThis[name]);",
  );
  assert.equal(result.ok, true);
  assert.deepEqual(result.value, ["undefined", "undefined", "undefined", "undefined"]);
  const imported = await execute("return await import('node:fs');");
  assert.equal(imported.ok, false);
});

test("synchronous loop and microtask loop terminate on deadline", async () => {
  for (const code of ["while (true) {}", "while (true) await null;"]) {
    const started = performance.now();
    const result = await execute(code);
    assert.equal(result.ok, false);
    assert.equal(result.error.kind, "timeout");
    assert.ok(performance.now() - started < 10000);
  }
});

test("unresolved promise and guest heap exhaustion fail", async () => {
  const unresolved = await execute("await new Promise(() => {});");
  assert.equal(unresolved.ok, false);
  const exhausted = await execute(
    "const values = []; while (true) values.push('x'.repeat(1024 * 1024));",
    { timeoutMs: 5000, memoryLimitBytes: 8 * 1024 * 1024 },
  );
  assert.equal(exhausted.ok, false);
});

test("scratch state is not restored when the caller supplies an empty store", async () => {
  const first = await execute("store('previous', true); return load('previous');");
  assert.equal(first.ok, true);
  assert.equal(first.value, true);
  const second = await execute("return load('previous');");
  assert.equal(second.ok, true);
  assert.equal(second.value, undefined);
});

// Characterization, not admission: retain the exact gap so a future adapter
// must close it before any real tool capability is connected.
test("records upstream output collection above DaVinci's one-MiB contract", async () => {
  const result = await execute("text('x'.repeat(2 * 1024 * 1024)); return 'done';");
  assert.equal(result.ok, true);
  const bytes = result.output.reduce((total, item) => total + Buffer.byteLength(item.text ?? ""), 0);
  assert.equal(bytes, 2 * 1024 * 1024);
  console.log(JSON.stringify({
    fact: "upstream-output-collection",
    bytes,
    davinciMaximumBytes: 1024 * 1024,
    admitted: false,
    actionRequired: "bounded adapter or upstream fix before C02 promotion",
  }));
});
