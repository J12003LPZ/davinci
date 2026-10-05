// Adapted from pi-codemode 1.0.2 runtime/worker.js, MIT, Mario Zechner.
// See licenses/pi-LICENSE. Keep guest code inside QuickJS/WASM.
import { parentPort, workerData } from "node:worker_threads";
import { JSException, MAX_STACK_SIZE, QuickJS } from "quickjs-wasi";
import { STRICT_PRELUDE_SOURCE } from "./strict-prelude.mjs";
import { isHostToWorkerMessage } from "./node_modules/@earendil-works/pi-codemode/dist/runtime/protocol.js";

const OUTPUT_CAP = 1048576;
const ARGUMENT_CAP = 65536;
const ERROR_CAP = 2048;
let terminal = false;
let outputBytes = 0;
let outputItems = 0;
let toolRequests = 0;
let metadataRequests = 0;
const post = (message) => parentPort?.postMessage(message);

function fail(message) {
  if (terminal) return;
  terminal = true;
  post({ type: "done", ok: false, error: JSON.stringify({ message }) });
}

// Check guest UTF16 length before copying out of the VM. At most 3*cap
// transient UTF8 bytes are inspected; rejected values never reach the queue
// or upstream collector. Counts are shared across text, console and return.
function boundedString(handle, cap) {
  if (handle === undefined || handle.isUndefined) return undefined;
  const length = handle.getProp("length");
  try {
    if (length.toNumber() > cap) {
      fail("LIMIT_EXCEEDED: guest value exceeds byte budget");
      return undefined;
    }
  } finally { length.dispose(); }
  const text = handle.toString();
  if (Buffer.byteLength(text, "utf8") > cap) {
    fail("LIMIT_EXCEEDED: guest value exceeds UTF8 byte budget");
    return undefined;
  }
  return text;
}

function discardOutput(memory) {
  return { fd_write(_fd, iovsPtr, iovsLen, nwrittenPtr) {
    const view = new DataView(memory.buffer);
    let written = 0;
    for (let i = 0; i < iovsLen; i++) written += view.getUint32(iovsPtr + i * 8 + 4, true);
    view.setUint32(nwrittenPtr, written, true);
    return 0;
  } };
}

async function main(data) {
  const interrupt = new Int32Array(data.interrupt);
  const vm = await QuickJS.create({
    wasm: data.wasm, memoryLimit: data.memoryLimitBytes,
    maxStackSize: MAX_STACK_SIZE,
    interruptHandler: () => terminal || Atomics.load(interrupt, 0) !== 0,
    wasi: discardOutput,
  });
  const bridge = vm.newFunction("bridge", (kind, a, b, c) => {
    if (terminal) return vm.undefined;
    switch (kind.toString()) {
      case "call":
      case "global": {
        const count = kind.toString() === "call" ? ++toolRequests : ++metadataRequests;
        if (count > 64) { fail("LIMIT_EXCEEDED: bridge request budget exhausted"); break; }
        const name = boundedString(b, 1024);
        const args = boundedString(c, ARGUMENT_CAP);
        if (!terminal) post({ type: "call", id: a.toNumber(),
          target: kind.toString() === "call" ? "tool" : "global", name, args });
        break;
      }
      case "output": {
        if (a.toString() !== "text") { fail("INVALID_INPUT: unsupported image output"); break; }
        if (++outputItems > 100000) { fail("LIMIT_EXCEEDED: output item budget exhausted"); break; }
        const text = boundedString(b, OUTPUT_CAP - outputBytes);
        if (!terminal) {
          outputBytes += Buffer.byteLength(text);
          post({ type: "output", item: { type: "text", text } });
        }
        break;
      }
      case "done": {
        const ok = a.toBoolean();
        const payload = boundedString(b, ok ? OUTPUT_CAP - outputBytes : ERROR_CAP);
        if (!terminal) {
          terminal = true;
          // Scratch writes are invocation-local, never copied or restored.
          post(ok ? { type: "done", ok: true, value: payload, writes: "[]" }
            : { type: "done", ok: false, error: payload });
        }
        break;
      }
      default: fail("PROTOCOL_ERROR: unknown guest bridge message");
    }
    return vm.undefined;
  });
  const api = vm.withScope((scope) => scope.escape(vm.callFunction(
    vm.evalCode(STRICT_PRELUDE_SOURCE, "codemode-prelude.js"), vm.undefined, bridge,
    vm.newString(JSON.stringify(data.tools)), vm.newString(JSON.stringify(data.globals)),
    vm.newString("{}"))));
  const settle = api.getProp("settle");
  const run = api.getProp("run");
  const stalled = api.getProp("stalled");
  const drain = () => {
    if (terminal) return;
    vm.executePendingJobs();
    if (!terminal) vm.callFunction(stalled, api).dispose();
  };
  parentPort?.on("message", (message) => {
    if (terminal || !isHostToWorkerMessage(message)) return;
    try {
      vm.withScope(() => vm.callFunction(settle, api, vm.newNumber(message.id),
        message.ok ? vm.true : vm.false,
        message.payload === undefined ? vm.undefined : vm.newString(message.payload)).dispose());
      drain();
    } catch { fail("SANDBOX_FAILED: guest job failed"); }
  });
  let fn;
  try { fn = vm.evalCode(`(async (tools, console) => {${data.code}\n})`, "codemode.js"); }
  catch (error) {
    if (!(error instanceof JSException)) throw error;
    fail("INVALID_INPUT: script failed to parse");
    return;
  }
  vm.callFunction(run, api, fn).dispose();
  fn.dispose();
  drain();
}

if (parentPort) main(workerData).catch(() => fail("SANDBOX_FAILED: guest worker failed"));
