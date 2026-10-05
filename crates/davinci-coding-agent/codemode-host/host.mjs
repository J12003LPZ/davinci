// The host owns only the VM and transport. Rust owns all capability authority.
import { CodemodeSandbox } from "@earendil-works/pi-codemode";
import { toCodemodeIdentifier } from "./node_modules/@earendil-works/pi-codemode/dist/identifier.js";
import { FrameDecoder, encodeFrame } from "./framing.mjs";

const decoder = new FrameDecoder();
let active;
let terminal = false;
let nextRequest = 0;
const pending = new Map();

function send(message) {
  if (!terminal) process.stdout.write(encodeFrame({ version: 1, ...message }));
}

// `runId` lets Rust attribute the failure even after the run was cleared.
function fatal(code = "PROTOCOL_ERROR", runId = active?.runId) {
  if (terminal) return;
  send({ type: "fatal", code, ...(runId === undefined ? {} : { runId }) });
  terminal = true;
  active?.controller.abort();
  for (const waiter of pending.values()) waiter.reject(new Error("protocol closed"));
  pending.clear();
  process.stdin.pause();
  process.exitCode = 1;
}

function request(type, name, args, signal) {
  if (terminal || !active || signal.aborted || pending.size >= 64) {
    return Promise.reject(new Error("request unavailable"));
  }
  const requestId = String(++nextRequest);
  return new Promise((resolve, reject) => {
    const abort = () => { pending.delete(requestId); reject(new Error("request cancelled")); };
    signal.addEventListener("abort", abort, { once: true });
    pending.set(requestId, { resolve, reject, signal, abort, type });
    send({ type, runId: active.runId, requestId, name, arguments: args });
  });
}

async function execute(message) {
  if (active || typeof message.runId !== "string" || message.runId.length > 128
    || typeof message.code !== "string" || Buffer.byteLength(message.code) > 65536
    || !Array.isArray(message.tools) || message.tools.length > 1024
    || !Number.isInteger(message.timeoutMs) || message.timeoutMs < 1 || message.timeoutMs > 300000
    || !Number.isInteger(message.memoryBytes) || message.memoryBytes < 8388608 || message.memoryBytes > 134217728) {
    throw new Error("invalid execution");
  }
  const aliases = new Set();
  const tools = message.tools.map((tool) => {
    if (typeof tool.name !== "string" || tool.name.length > 1024
      || aliases.has(toCodemodeIdentifier(tool.name))) throw new Error("ambiguous tool alias");
    aliases.add(toCodemodeIdentifier(tool.name));
    return { name: tool.name, description: "", execute: (args, { signal }) => request("tool_call", tool.name, args, signal) };
  });
  const controller = new AbortController();
  const sandbox = new CodemodeSandbox({
    workerUrl: new URL("./bounded-worker.mjs", import.meta.url),
    tools, timeoutMs: message.timeoutMs, memoryLimitBytes: message.memoryBytes,
    globals: ["search", "describe"].map((name) => ({ name: `codemode.${name}`,
      execute: (args, { signal }) => request("metadata_query", name, args, signal) })),
  });
  active = { runId: message.runId, controller };
  try {
    const result = await sandbox.execute(message.code, { store: {}, signal: controller.signal });
    // No persistent scratch writes or guest-authored journal state cross this boundary.
    delete result.storeWrites;
    send({ type: "finished", runId: message.runId, result });
  } finally {
    await sandbox.close();
    for (const waiter of pending.values()) waiter.reject(new Error("run finished"));
    pending.clear();
    active = undefined;
  }
}

function receive(message) {
  if (message.type === "execute") {
    // A finished frame can outgrow MAX_FRAME once JSON escaping expands output.
    void execute(message).catch((error) => fatal(
      error?.message === "frame limit" ? "LIMIT_EXCEEDED" : "PROTOCOL_ERROR", message.runId));
    return;
  }
  if (!active || message.runId !== active.runId) throw new Error("wrong run");
  if (message.type === "cancel") { active.controller.abort(); return; }
  const waiter = pending.get(message.requestId);
  if (!waiter || (message.type !== "tool_result" && message.type !== "metadata_result")
    || (waiter.type === "tool_call") !== (message.type === "tool_result")
    || typeof message.ok !== "boolean") throw new Error("unexpected reply");
  pending.delete(message.requestId);
  waiter.signal.removeEventListener("abort", waiter.abort);
  if (message.ok) waiter.resolve(message.value);
  else {
    const error = message.value?.error;
    if (!error || typeof error.code !== "string" || typeof error.message !== "string") {
      fatal();
      waiter.reject(new Error("PROTOCOL_ERROR: malformed broker error"));
      return;
    }
    waiter.reject(new Error(JSON.stringify(error)));
  }
}

process.stdin.on("data", (chunk) => {
  try { for (const message of decoder.push(chunk)) receive(message); }
  catch { fatal(); }
});
process.stdin.on("end", () => {
  try { decoder.finish(); } catch { fatal(); }
  active?.controller.abort();
});
process.stdin.on("error", () => fatal());
process.stdout.on("error", () => { terminal = true; active?.controller.abort(); });
send({ type: "hello", nodeVersion: process.version, protocolVersion: 1 });
