import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { FrameDecoder, encodeFrame } from "./framing.mjs";

function start(flags = []) {
  const child = spawn(process.execPath, [...flags, fileURLToPath(new URL("./host.mjs", import.meta.url))], { stdio: ["pipe", "pipe", "pipe"] });
  const decoder = new FrameDecoder();
  const messages = [];
  const waiters = [];
  child.stdout.on("data", (data) => {
    for (const message of decoder.push(data)) {
      const waiter = waiters.shift();
      if (waiter) waiter(message); else messages.push(message);
    }
  });
  const next = () => messages.length ? Promise.resolve(messages.shift())
    : new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("host response timeout")), 5000);
      waiters.push((message) => { clearTimeout(timer); resolve(message); });
    });
  return { child, next, send: (message) => child.stdin.write(encodeFrame({ version: 1, ...message })) };
}

test("framed host executes a fresh bounded VM with no tools", async () => {
  const host = start();
  try {
    assert.equal((await host.next()).type, "hello");
    host.send({ type: "execute", runId: "fixture", code: "return 42;", tools: [], timeoutMs: 1000, memoryBytes: 67108864 });
    const finished = await host.next();
    assert.equal(finished.type, "finished");
    assert.equal(finished.runId, "fixture");
    assert.equal(finished.result.value, 42);
  } finally { host.child.kill(); }
});

test("framed host correlates calls through the Rust-side fixture peer", async () => {
  const host = start();
  try {
    await host.next();
    host.send({ type: "execute", runId: "broker", code: "return await tools.fixture({ id: 7 });", tools: [{ name: "fixture" }], timeoutMs: 1000, memoryBytes: 67108864 });
    const call = await host.next();
    assert.equal(call.type, "tool_call");
    assert.deepEqual(call.arguments, { id: 7 });
    host.send({ type: "tool_result", runId: "broker", requestId: call.requestId, ok: true, value: { matched: 7 } });
    const finished = await host.next();
    assert.deepEqual(finished.result.value, { matched: 7 });
  } finally { host.child.kill(); }
});

test("cancel terminates a synchronous runaway guest", async () => {
  const host = start();
  try {
    await host.next();
    host.send({ type: "execute", runId: "cancel", code: "while(true) {}", tools: [], timeoutMs: 10000, memoryBytes: 67108864 });
    host.send({ type: "cancel", runId: "cancel" });
    const finished = await host.next();
    assert.equal(finished.type, "finished");
    assert.equal(finished.result.ok, false);
    assert.equal(finished.result.error.kind, "aborted");
  } finally { host.child.kill(); }
});

test("unexpected replies close the transport", async () => {
  const host = start();
  try {
    await host.next();
    host.send({ type: "tool_result", runId: "forged", requestId: "1", ok: true, value: {} });
    assert.equal((await host.next()).type, "fatal");
  } finally { host.child.kill(); }
});

test("broker errors preserve stable codes inside the guest", async () => {
  const host = start();
  try {
    await host.next();
    host.send({type:"execute", runId:"denied", code:"try { await tools.fixture({}); } catch (error) { return {code:error.code, ref:error.operationRef}; }",
      tools:[{name:"fixture"}], timeoutMs:1000, memoryBytes:67108864});
    const call = await host.next();
    host.send({type:"tool_result",runId:"denied",requestId:call.requestId,ok:false,
      value:{error:{code:"DENIED",message:"fixture denied",operationRef:"op-fixture"}}});
    assert.deepEqual((await host.next()).result.value,{code:"DENIED",ref:"op-fixture"});
  } finally { host.child.kill(); }
});

test("duplicate replies close transport after the first correlation is consumed", async () => {
  const host = start();
  try {
    await host.next();
    host.send({type:"execute",runId:"duplicate",code:"await tools.fixture({}); await tools.fixture({});",
      tools:[{name:"fixture"}],timeoutMs:1000,memoryBytes:67108864});
    const call = await host.next();
    const reply = {type:"tool_result",runId:"duplicate",requestId:call.requestId,ok:true,value:{}};
    host.send(reply);
    host.send(reply);
    assert.equal((await host.next()).type,"fatal");
  } finally { host.child.kill(); }
});

test("alias collisions reject the run before executing code", async () => {
  const host = start();
  try {
    await host.next();
    host.send({ type: "execute", runId: "collision", code: "return 42", tools: [{name:"a-b"},{name:"a_b"}], timeoutMs: 1000, memoryBytes: 67108864 });
    assert.equal((await host.next()).type, "fatal");
  } finally { host.child.kill(); }
});

test("collector overflow stays bounded across the outer transport", async () => {
  const host = start();
  try {
    await host.next();
    host.send({ type: "execute", runId: "overflow", code: "text('x'.repeat(2097152));", tools: [], timeoutMs: 1000, memoryBytes: 67108864 });
    const finished = await host.next();
    assert.equal(finished.result.ok, false);
    assert.match(finished.result.error.message, /LIMIT_EXCEEDED/);
    assert.equal(finished.result.output.length, 0);
  } finally { host.child.kill(); }
});

test("escaped output beyond the frame limit is reported as a limit, not a protocol fault", async () => {
  const host = start();
  try {
    await host.next();
    // 1MB of control characters fits the collector but JSON-escapes to ~6MB.
    host.send({ type: "execute", runId: "escaped", code: "text('\u0001'.repeat(1000000)); return 1;", tools: [], timeoutMs: 5000, memoryBytes: 67108864 });
    const fatal = await host.next();
    assert.equal(fatal.type, "fatal");
    assert.equal(fatal.code, "LIMIT_EXCEEDED");
    assert.equal(fatal.runId, "escaped");
  } finally { host.child.kill(); }
});

test("host runs under the Node permission model with read-only asset access", async () => {
  const root = fileURLToPath(new URL(".", import.meta.url));
  const host = start(["--permission", "--allow-worker", `--allow-fs-read=${root}`]);
  try {
    assert.equal((await host.next()).type, "hello");
    host.send({ type: "execute", runId: "permission", code: "return 42;", tools: [], timeoutMs: 1000, memoryBytes: 67108864 });
    const finished = await host.next();
    assert.equal(finished.type, "finished");
    assert.equal(finished.result.value, 42);
  } finally { host.child.kill(); }
});
