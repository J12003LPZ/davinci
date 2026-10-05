import test from "node:test";
import assert from "node:assert/strict";
import { FrameDecoder, encodeFrame } from "./framing.mjs";

test("frames survive arbitrary partial reads", () => {
  const wire = encodeFrame({ version: 1, type: "hello", text: "界" });
  const decoder = new FrameDecoder();
  const received = [];
  for (const byte of wire) received.push(...decoder.push(Buffer.from([byte])));
  assert.deepEqual(received, [{ version: 1, type: "hello", text: "界" }]);
  decoder.finish();
});

test("rejects oversized length before allocating a body", () => {
  const header = Buffer.alloc(4);
  header.writeUInt32BE(2097153);
  const decoder = new FrameDecoder();
  assert.throws(() => decoder.push(header), /frame limit/);
  assert.throws(() => decoder.push(Buffer.from("x")), /closed/);
});

test("rejects malformed JSON, UTF8, version and truncated frames", () => {
  for (const body of [Buffer.from("{"), Buffer.from([0xff]), Buffer.from('{"version":2}')]) {
    const header = Buffer.alloc(4); header.writeUInt32BE(body.length);
    assert.throws(() => new FrameDecoder().push(Buffer.concat([header, body])));
  }
  const decoder = new FrameDecoder();
  decoder.push(Buffer.from([0, 0]));
  assert.throws(() => decoder.finish(), /truncated/);
});

test("multiple frames preserve boundaries and outgoing limits", () => {
  const messages = [{ version: 1, type: "hello" }, { version: 1, type: "finished" }];
  assert.deepEqual(new FrameDecoder().push(Buffer.concat(messages.map(encodeFrame))), messages);
  assert.throws(() => encodeFrame({ version: 1, text: "x".repeat(2097152) }), /frame limit/);
});
