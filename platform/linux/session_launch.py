#!/usr/bin/env python3
"""Launch broker: fixed trusted programs receive authority, callers never do."""
import array
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import threading
import select
import shutil

from session_authority import sealed_authority, peer_pid
from session_protocol import MAX_FRAME, private_directory, require_peer

BASE = Path(__file__).resolve().parent
# No interpreter/module options, arbitrary executables, loader variables or
# plugin-controlled entrypoints are accepted by the authority broker.
ENVIRONMENT = frozenset(('DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY', 'DBUS_SESSION_BUS_ADDRESS',
    'LANG', 'LANGUAGE', 'LC_ALL', 'LC_MESSAGES', 'GDK_BACKEND', 'GTK_A11Y', 'GSK_RENDERER',
    'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_DATA_HOME', 'MOCHI_BUBBLE_SOCKET',
    'MOCHI_MANAGER_V2_NON_UNIQUE', 'MOCHI_MANAGER_NON_UNIQUE',
    'MOCHI_RESTORE_PANEL', 'MOCHI_RENDERER_PID', 'MOCHI_THEME_CONFIG', 'MOCHI_THEME_TRANSLATION', 'MOCHI_RUNTIME_GENERATION',
    'MOCHI_MCP_PORT', 'MOCHI_MCP_PATH', 'MOCHI_MCP_TOKEN'))


def trusted_command(role, arguments):
    if role in ('manager', 'legacy') and not arguments:
        from manager_launcher import resolved_command
        command = resolved_command(role == 'legacy')
        return [command[0], '-I', '-B', *command[1:]], 'manager'
    if role == 'host' and len(arguments) == 1:
        roles = json.loads(arguments[0])
        if (not isinstance(roles, list) or len(roles) > 4 or
            any(not isinstance(row, list) or len(row) != 2 or type(row[0]) is not int or
                not isinstance(row[1], str) or len(row[1]) > 128 for row in roles)):
            raise ValueError('Invalid renderer roles.')
        return ['/usr/bin/python3', '-I', '-B', str(BASE/'bubble_host.py'), json.dumps(roles)], 'host'
    if role == 'card-worker' and len(arguments) == 1 and isinstance(arguments[0], str) and len(arguments[0]) < 4096:
        from mochi_paths import DATA
        return [shutil.which('node') or '/usr/bin/node', str(DATA/'plugins/mochi-chatgpt/http-worker.mjs'), str(Path(arguments[0]).resolve())], 'card'
    if role in ('card-http', 'card-stdio') and not arguments:
        from mochi_paths import DATA
        entry = DATA/'plugins/mochi-chatgpt'/('http.mjs' if role == 'card-http' else 'mcp.mjs')
        return [shutil.which('node') or '/usr/bin/node', str(entry)], 'card'
    raise ValueError('Unsupported trusted entrypoint.')


class LaunchBroker:
    def __init__(self, authority):
        self.authority = authority
        self.path = private_directory()/'session-launch.sock'
        self.path.unlink(missing_ok=True)
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.bind(str(self.path)); os.chmod(self.path, 0o600)
        self.socket.listen(16)
        self.closed = False
        self.slots = threading.BoundedSemaphore(32)
        threading.Thread(target=self.run, name='session-launch', daemon=True).start()

    def run(self):
        while not self.closed:
            try:
                connection, _ = self.socket.accept()
            except OSError:
                return
            if not self.slots.acquire(blocking=False):
                connection.close()
                continue
            threading.Thread(target=self.launch, args=(connection,), daemon=True).start()

    def launch(self, connection):
        descriptors = []
        grant = None
        try:
            require_peer(connection)
            peer = Path(f'/proc/{peer_pid(connection)}/status').read_text()
            if 'NoNewPrivs:\t1' in peer:
                raise PermissionError('Confined plugin cannot request a trusted process.')
            connection.settimeout(3)
            raw, ancillary, flags, _ = connection.recvmsg(MAX_FRAME, socket.CMSG_SPACE(3*4))
            for level, kind, payload in ancillary:
                if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                    values = array.array('i'); values.frombytes(payload); descriptors.extend(values)
            if flags & (socket.MSG_TRUNC | socket.MSG_CTRUNC) or len(descriptors) != 3:
                raise ValueError('Expected three stdio descriptors.')
            message = json.loads(raw)
            command, role = trusted_command(message['role'], message.get('arguments', []))
            environment = {key: value for key, value in os.environ.items()
                           if key not in ENVIRONMENT and key not in ('PYTHONPATH', 'PYTHONHOME', 'LD_PRELOAD', 'LD_LIBRARY_PATH', 'NODE_OPTIONS', 'MOCHI_SESSION_FD')}
            environment.update({key: value for key, value in message.get('environment', {}).items()
                                if key in ENVIRONMENT and isinstance(value, str) and len(value) < 8192})
            if message['role'] == 'legacy': environment['MOCHI_MANAGER_LEGACY'] = '1'
            grant = self.authority.issue(role)
            fd = sealed_authority(grant)
            environment['MOCHI_SESSION_FD'] = str(fd)
            # -I disables PYTHONPATH; trusted entrypoints insert their own module
            # directory explicitly before imports (no cwd module search).
            try:
                child = subprocess.Popen(command, env=environment, cwd='/',
                    stdin=descriptors[0], stdout=descriptors[1], stderr=descriptors[2],
                    pass_fds=(fd,), start_new_session=True)
                self.authority.grants[grant['capability']] = (role, 'chatgpt', child.pid)
            finally:
                os.close(fd)
            connection.sendall(json.dumps({'pid': child.pid}).encode()+b'\n')
            code = child.wait()
            connection.sendall(json.dumps({'exit': code}).encode()+b'\n')
        except (OSError, ValueError, KeyError, TypeError):
            try: connection.sendall(b'{"error":"session.launch_rejected"}\n')
            except OSError: pass
        finally:
            if grant: self.authority.grants.pop(grant['capability'], None)
            for fd in descriptors: os.close(fd)
            connection.close()
            self.slots.release()

    def close(self):
        self.closed = True
        self.socket.close()
        self.path.unlink(missing_ok=True)


def launch(role, arguments=()):
    from session_client import ensure_sessiond
    ensure_sessiond()
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(str(private_directory()/'session-launch.sock'))
        require_peer(connection)
        value = {'role': role, 'arguments': list(arguments),
                 'environment': {key: value for key, value in os.environ.items() if key in ENVIRONMENT}}
        connection.sendmsg([json.dumps(value).encode()], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', (0, 1, 2)))])
        with connection.makefile('rb') as stream:
            response = json.loads(stream.readline(MAX_FRAME))
            if 'pid' not in response: raise PermissionError('Trusted session launch rejected.')
            pid = response['pid']
            def forward(signum, _frame):
                try: os.kill(pid, signum)
                except ProcessLookupError: pass
            signal.signal(signal.SIGTERM, forward); signal.signal(signal.SIGINT, forward)
            raw = stream.readline(MAX_FRAME)
            if not raw:
                # A crashed daemon must not kill its surviving trusted UI.
                try:
                    descriptor = os.pidfd_open(pid)
                    try: select.select([descriptor], [], [])
                    finally: os.close(descriptor)
                except ProcessLookupError:
                    pass
                return 0
            response = json.loads(raw)
            return max(0, response.get('exit', 1))


if __name__ == '__main__':
    raise SystemExit(launch(sys.argv[1], sys.argv[2:]))
