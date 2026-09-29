"""Small management view for one-shot Codex completion reminders."""
import json
import struct
import sys

from watch import pending_labels


def read_exact(size):
    data = bytearray()
    while len(data) < size:
        chunk = sys.stdin.buffer.read(size - len(data))
        if not chunk:
            return None
        data.extend(chunk)
    return bytes(data)


def send(identity, kind, **fields):
    payload = json.dumps({'id': identity, 'type': kind, **fields}, ensure_ascii=False).encode()
    sys.stdout.buffer.write(struct.pack('=I', len(payload)) + payload)
    sys.stdout.buffer.flush()


while header := read_exact(4):
    size = struct.unpack('=I', header)[0]
    if not 0 < size <= 256 * 1024:
        break
    payload = read_exact(size)
    if payload is None:
        break
    request = json.loads(payload)
    labels = pending_labels()
    text = ('等待完成：\n' + '\n'.join('• ' + label for label in labels[:8])) if labels else '没有等待中的 Codex 任务。'
    text += '\n\n在要提醒的 Codex 会话里说：“完成当前任务时用 Mochi 提醒我。”'
    send(request['id'], 'view', content={'title': 'Codex 提醒', 'text': text})
    send(request['id'], 'done')
