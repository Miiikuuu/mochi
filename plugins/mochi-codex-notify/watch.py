#!/usr/bin/env python3
"""One-shot Codex completion registration and Mochi delivery.

Codex invokes this file with its agent-turn-complete JSON as argv[1].
Run `watch.py arm --label ...` inside a selected Codex thread to register it.
"""
import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import re
import socket
import stat
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
MODULES = ROOT/'platform/linux'
if not MODULES.is_dir():
    MODULES = ROOT.parents[1]/'libexec/mochi/platform/linux'
sys.path.insert(0, str(MODULES))
from desktop_ipc import socket_path  # noqa: E402

THREAD_ID = re.compile(r'[A-Za-z0-9_-]{8,128}\Z')
PLUGIN = 'codex-notify'


def state_dir():
    override = os.environ.get('MOCHI_PLUGIN_STATE_DIR') or os.environ.get('MOCHI_CODEX_NOTIFY_STATE_DIR')
    return Path(override) if override else Path(os.environ.get('XDG_STATE_HOME', str(Path.home() / '.local/state'))) / 'mochi' / 'plugins' / 'codex-notify'


@contextmanager
def locked_state():
    directory = state_dir()
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    if directory.is_symlink() or directory.stat().st_uid != os.getuid():
        raise ValueError('提醒状态目录不属于当前用户。')
    directory.chmod(0o700)
    fd = os.open(directory / 'watches.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'r+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        path = directory / 'watches.json'
        if path.is_symlink():
            raise ValueError('提醒状态文件不能是链接。')
        legacy = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))/'mochi/codex-notify/watches.json'
        source = path if path.exists() else legacy
        watches = json.loads(source.read_text()) if source.exists() else {}
        if not isinstance(watches, dict):
            raise ValueError('提醒状态文件格式无效。')
        yield watches
        with tempfile.NamedTemporaryFile('w', encoding='utf-8', dir=directory, prefix='.watches-', delete=False) as stream:
            temporary = Path(stream.name)
            os.fchmod(stream.fileno(), 0o600)
            json.dump(watches, stream, ensure_ascii=False)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(path)


def request_desktop(request):
    path = socket_path()
    info = path.lstat()
    parent = path.parent.lstat()
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid() or not stat.S_ISDIR(parent.st_mode) or parent.st_uid != os.getuid() or parent.st_mode & 0o077:
        raise ValueError('Mochi 桌面连接无效。')
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(2.5)
        client.connect(str(path))
        peer = client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
        if int.from_bytes(peer[4:8], sys.byteorder) != os.getuid():
            raise ValueError('Mochi 桌面连接不属于当前用户。')
        client.sendall(json.dumps(request, ensure_ascii=False).encode() + b'\n')
        data = bytearray()
        while b'\n' not in data:
            chunk = client.recv(8192)
            if not chunk or len(data) + len(chunk) > 65536:
                raise ValueError('Mochi 未确认收到提醒。')
            data.extend(chunk)
    result = json.loads(data.split(b'\n', 1)[0])
    if not isinstance(result, dict) or result.get('error'):
        raise ValueError(str(result.get('error', 'Mochi 未确认收到提醒。'))[:300])
    return result


def selected_thread(explicit=None):
    identity = explicit or os.environ.get('CODEX_THREAD_ID') or os.environ.get('CODEX_SESSION_ID')
    if not isinstance(identity, str) or not THREAD_ID.fullmatch(identity):
        raise ValueError('请从需要提醒的本机 Codex 会话中运行此命令。')
    return identity


def arm(label=None, thread=None):
    identity = selected_thread(thread)
    status = request_desktop({'type': 'theme_status'})
    if PLUGIN not in status.get('plugins', []):
        raise ValueError('请先在当前 Mochi Theme 启用“Codex 提醒”插件。')
    if not any(role.get('alive') and role.get('window') == status.get('primary') for role in status.get('roles', [])):
        raise ValueError('请先打开 Mochi 主角色。')
    value = label or Path(os.getcwd()).name or 'Codex 任务'
    value = ' '.join(value.split())[:80]
    if not value:
        raise ValueError('提醒名称不能为空。')
    with locked_state() as watches:
        watches[identity] = {'label': value, 'created': int(time.time())}
    return value


def pending_labels():
    with locked_state() as watches:
        return [item['label'] for item in watches.values() if isinstance(item, dict) and isinstance(item.get('label'), str)]


def cancel(thread=None):
    identity = selected_thread(thread)
    with locked_state() as watches:
        return watches.pop(identity, None) is not None


def complete(event):
    if not isinstance(event, dict) or event.get('type') != 'agent-turn-complete':
        return False
    identity = event.get('thread-id')
    if not isinstance(identity, str) or not THREAD_ID.fullmatch(identity):
        return False
    with locked_state() as watches:
        watch = watches.pop(identity, None)
    if not watch:
        return False
    label = watch.get('label', 'Codex 任务') if isinstance(watch, dict) else 'Codex 任务'
    request_desktop({'type': 'plugin_message', 'plugin': PLUGIN, 'title': 'Codex 已完成',
                     'text': f'“{label}”已完成。', 'ttl_ms': 20000, 'system_notify': True})
    return True


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    parser = argparse.ArgumentParser(description='Mochi 的 Codex 单次完成提醒')
    parser.add_argument('operation', nargs='?', default='list')
    parser.add_argument('--label')
    parser.add_argument('--thread-id')
    args = parser.parse_args(argv)
    try:
        if args.operation == 'arm':
            print('已订阅一次完成提醒：' + arm(args.label, args.thread_id))
        elif args.operation == 'cancel':
            print('已取消。' if cancel(args.thread_id) else '没有待取消的提醒。')
        elif args.operation == 'list':
            print('\n'.join(pending_labels()) or '没有等待中的任务。')
        elif args.operation.startswith('{'):
            complete(json.loads(args.operation))
        else:
            parser.error('不支持的操作')
    except (OSError, ValueError, json.JSONDecodeError) as error:
        # A failed notification must not fail the completed Codex task.
        if args.operation.startswith('{'):
            print('Mochi 提醒未送达：' + str(error), file=sys.stderr)
            return 0
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
