// Pin-dependent adaptation. Capture intrinsics before any guest code executes.
import { PRELUDE_SOURCE } from "./node_modules/@earendil-works/pi-codemode/dist/runtime/prelude-source.js";
const original = "return value === undefined ? undefined : stringify(value);";
if (PRELUDE_SOURCE.split(original).length !== 2) throw new Error("unrecognized admitted prelude");
export const STRICT_PRELUDE_SOURCE = PRELUDE_SOURCE.replace(original, `
    if (value === undefined) return undefined;
    return stringify(validateJson(value, 0, []));
`).replace('const stringify = JSON.stringify;', `
  const stringify = JSON.stringify;
  const jsonPrototype = Object.prototype;
  const jsonArray = Array.isArray;
  const jsonPrototypeOf = Object.getPrototypeOf;
  const jsonKeys = Reflect.ownKeys;
  const jsonDescriptor = Object.getOwnPropertyDescriptor;
  const jsonCreate = Object.create;
  const jsonDefine = Object.defineProperty;
  const jsonSetPrototype = Object.setPrototypeOf;
  const jsonString = String;
  const jsonFinite = Number.isFinite;
  const jsonInteger = Number.isInteger;
  const jsonSafe = Number.isSafeInteger;
  function validateJson(value, depth, ancestors) {
    if (depth > 64) throw new TypeError("LIMIT_EXCEEDED: JSON depth");
    if (value === null || typeof value === "string" || typeof value === "boolean") return value;
    if (typeof value === "number") {
      if (!jsonFinite(value) || (jsonInteger(value) && !jsonSafe(value))) {
        throw new TypeError("INVALID_INPUT: unsafe JSON number");
      }
      return value;
    }
    if (typeof value !== "object") throw new TypeError("INVALID_INPUT: unsupported JSON value");
    for (let i = 0; i < ancestors.length; i++) {
      if (ancestors[i] === value) throw new TypeError("INVALID_INPUT: cyclic JSON");
    }
    const array = jsonArray(value);
    if (!array && jsonPrototypeOf(value) !== jsonPrototype && jsonPrototypeOf(value) !== null) {
      throw new TypeError("INVALID_INPUT: non-JSON object");
    }
    jsonDefine(ancestors, jsonString(depth), {value, writable:true, configurable:true});
    const copy = array ? jsonSetPrototype([], null) : jsonCreate(null);
    const keys = jsonKeys(value);
    if (array) {
      const length = jsonDescriptor(value, "length").value;
      for (let i = 0; i < length; i++) {
        if (!jsonDescriptor(value, jsonString(i))) throw new TypeError("INVALID_INPUT: sparse JSON array");
      }
      copy.length = length;
    }
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (array && key === "length") continue;
      if (typeof key !== "string") throw new TypeError("INVALID_INPUT: symbol JSON key");
      const descriptor = jsonDescriptor(value, key);
      if (!descriptor || descriptor.get || descriptor.set) throw new TypeError("INVALID_INPUT: JSON accessor");
      if (!descriptor.enumerable || (array && (key === "" || jsonString(+key) !== key || +key < 0 || !jsonInteger(+key)))) {
        throw new TypeError("INVALID_INPUT: non-JSON property");
      }
      const child = validateJson(descriptor.value, depth + 1, ancestors);
      jsonDefine(copy, key, {value:child, enumerable:true, writable:true, configurable:true});
    }
    ancestors.length = depth;
    return copy;
  }
`).replace('entry.reject(new ErrorCtor(payload));', `
    let error = new ErrorCtor(payload);
    try {
      const description = parse(payload);
      const allowed = ["UNAVAILABLE", "INVALID_INPUT", "DENIED", "CANCELLED", "TIMEOUT",
        "LIMIT_EXCEEDED", "TOOL_FAILED", "STALE_CAPABILITY", "INCOMPLETE_DATA",
        "PROTOCOL_ERROR", "SANDBOX_FAILED", "RECOVERY_REQUIRED"];
      for (let i = 0; i < allowed.length; i++) {
        if (description.code === allowed[i] && typeof description.message === "string") {
          error = new ErrorCtor(description.message);
          jsonDefine(error, "code", {value:description.code, enumerable:true});
          jsonDefine(error, "operationRef", {value:description.operationRef ?? null, enumerable:true});
          break;
        }
      }
    } catch {}
    entry.reject(error);
`);
