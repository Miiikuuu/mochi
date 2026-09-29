// Chrome native messaging: native-endian uint32 length followed by UTF-8 JSON.
import { endianness } from 'node:os';
const LE = endianness() === 'LE';
export const MAX_FRAME = 256 * 1024;
export function encode(message) {
  const body = Buffer.from(JSON.stringify(message));
  if (body.length > MAX_FRAME) throw new Error('消息过大。');
  const header = Buffer.alloc(4);
  if (LE) header.writeUInt32LE(body.length); else header.writeUInt32BE(body.length);
  return Buffer.concat([header, body]);
}
export class Decoder {
  buffer = Buffer.alloc(0);
  push(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    const messages = [];
    while (this.buffer.length >= 4) {
      const length = LE ? this.buffer.readUInt32LE(0) : this.buffer.readUInt32BE(0);
      if (!length || length > MAX_FRAME) throw new Error('消息长度无效。');
      if (this.buffer.length < length + 4) break;
      messages.push(JSON.parse(this.buffer.subarray(4, length + 4).toString('utf8')));
      this.buffer = this.buffer.subarray(length + 4);
    }
    return messages;
  }
  finish() { if (this.buffer.length) throw new Error('消息不完整。'); }
}
