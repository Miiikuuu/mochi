"""Mochi-side view; chat and inference stay in ChatGPT."""
import json
import struct
import sys


def read_exact(size):
    result = bytearray()
    while len(result) < size:
        chunk = sys.stdin.buffer.read(size-len(result))
        if not chunk:
            return None
        result.extend(chunk)
    return bytes(result)


def send(identity, kind, **fields):
    payload = json.dumps({'id': identity, 'type': kind, **fields}, ensure_ascii=False).encode()
    sys.stdout.buffer.write(struct.pack('=I', len(payload))+payload)
    sys.stdout.buffer.flush()


while header := read_exact(4):
    size = struct.unpack('=I', header)[0]
    if not 0 < size <= 256*1024:
        break
    payload = read_exact(size)
    if payload is None:
        break
    request = json.loads(payload)
    send(request['id'], 'view', content={'title': 'ChatGPT', 'text': '在 ChatGPT 中调用 mochi_connect，点击“连接此对话”，即可在桌宠气泡里回复。\n\n对话和上下文保留在 ChatGPT，请保持连接卡片打开。', 'status': '等待连接'})
    send(request['id'], 'done')
