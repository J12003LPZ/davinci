// Protocol v1: a four-byte big-endian byte length followed by strict UTF8 JSON.
const MAX_FRAME = 2097152;
const utf8 = new TextDecoder("utf-8", { fatal: true });

export function encodeFrame(message) {
  const body = Buffer.from(JSON.stringify(message), "utf8");
  if (body.length === 0 || body.length > MAX_FRAME) throw new Error("frame limit");
  const header = Buffer.alloc(4);
  header.writeUInt32BE(body.length);
  return Buffer.concat([header, body]);
}

export class FrameDecoder {
  #header = Buffer.alloc(4);
  #headerBytes = 0;
  #body;
  #bodyBytes = 0;
  #closed = false;

  push(chunk) {
    if (this.#closed) throw new Error("decoder closed");
    const messages = [];
    try {
      let offset = 0;
      while (offset < chunk.length) {
        if (!this.#body) {
          const count = Math.min(4 - this.#headerBytes, chunk.length - offset);
          chunk.copy(this.#header, this.#headerBytes, offset, offset + count);
          offset += count;
          this.#headerBytes += count;
          if (this.#headerBytes !== 4) continue;
          const length = this.#header.readUInt32BE();
          if (length === 0 || length > MAX_FRAME) throw new Error("frame limit");
          this.#body = Buffer.alloc(length);
        }
        const count = Math.min(this.#body.length - this.#bodyBytes, chunk.length - offset);
        chunk.copy(this.#body, this.#bodyBytes, offset, offset + count);
        this.#bodyBytes += count;
        offset += count;
        if (this.#bodyBytes === this.#body.length) {
          const message = JSON.parse(utf8.decode(this.#body));
          if (message === null || typeof message !== "object" || Array.isArray(message)
            || message.version !== 1 || typeof message.type !== "string") {
            throw new Error("invalid protocol envelope");
          }
          messages.push(message);
          this.#body = undefined;
          this.#bodyBytes = 0;
          this.#headerBytes = 0;
        }
      }
      return messages;
    } catch (error) {
      this.#closed = true;
      this.#body = undefined;
      throw error;
    }
  }

  finish() {
    this.#closed = true;
    if (this.#body || this.#headerBytes) throw new Error("truncated frame");
  }
}
