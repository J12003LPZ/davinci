'use strict';
const { EventEmitter } = require('node:events');
const MAX_FRAME = 1024 * 1024;
class FramedProtocol extends EventEmitter {
  constructor(input, output) {
    super();
    this.output = output;
    this.buffer = Buffer.alloc(0);
    input.on('data', data => this.consume(data));
    input.on('end', () => this.emit('closed'));
  }
  consume(data) {
    if (this.buffer.length + data.length > MAX_FRAME + 4) {
      this.emit('error', new Error('protocol buffer limit')); return;
    }
    this.buffer = Buffer.concat([this.buffer, data]);
    while (this.buffer.length >= 4) {
      const size = this.buffer.readUInt32BE(0);
      if (!size || size > MAX_FRAME) { this.emit('error', new Error('protocol frame limit')); return; }
      if (this.buffer.length < size + 4) return;
      const frame = this.buffer.subarray(4, size + 4);
      this.buffer = this.buffer.subarray(size + 4);
      try {
        const message = JSON.parse(frame.toString('utf8'));
        if (!message || message.version !== 1 || typeof message.id !== 'string' ||
            message.id.length > 64 || !/^[a-zA-Z0-9-]+$/.test(message.id)) throw new Error('invalid envelope');
        this.emit('message', message);
      } catch { this.emit('error', new Error('invalid protocol message')); return; }
    }
  }
  send(message) {
    const body = Buffer.from(JSON.stringify({ ...message, version: 1 }));
    if (body.length > MAX_FRAME) throw new Error('protocol frame limit');
    const header = Buffer.alloc(4); header.writeUInt32BE(body.length);
    this.output.write(Buffer.concat([header, body]));
  }
}
module.exports = { FramedProtocol, MAX_FRAME };
