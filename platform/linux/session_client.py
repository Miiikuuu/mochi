"""Toolkit-free sessiond client and disposable state subscription."""
import fcntl
import json
import os
from pathlib import Path
import socket
import subprocess
import threading
import time
import sys

from session_authority import inherited_authority
from session_protocol import MAX_FRAME, VERSION, connect, private_directory

DISCONNECTED = {'connected': False, 'conversation': '', 'heartbeat_age': None, 'heartbeat_at': None,
                'phase': 'idle', 'response_received': False}
EMPTY_BINDING = {'key': None, 'connected': False, 'enabled': False, 'status': 'Disconnected',
                 'clear_id': '', 'waiting': False}


def request(operation, **arguments):
    frame = json.dumps({'version': VERSION, 'operation': operation, **inherited_authority(), **arguments}, allow_nan=False).encode()+b'\n'
    if len(frame) > MAX_FRAME:
        raise ValueError('Session request is too large.')
    with connect() as connection:
        connection.sendall(frame)
        with connection.makefile('rb') as stream:
            raw = stream.readline(MAX_FRAME+1)
        if len(raw) > MAX_FRAME or not raw.endswith(b'\n'):
            raise ConnectionError('Invalid session response.')
        reply = json.loads(raw)
        if not isinstance(reply, dict) or reply.get('version') != VERSION:
            raise ConnectionError('session.version_mismatch: upgrade or restart the client and service.')
        if reply.get('error'):
            raise ValueError(reply['error'].get('code', 'session.rejected')+': '+reply['error'].get('message', 'Session rejected.'))
        return reply['result']


def status(scope="chatgpt"):
    try:
        frame = request('status', scope=scope)
        frame['_observed_at'] = time.monotonic()
        return frame
    except (OSError, ValueError, KeyError) as error:
        return {'version': VERSION, 'generation': None, 'revision': 0, '_observed_at': time.monotonic(),
                'state': {**DISCONNECTED, 'service_error': str(error)[:256]}, 'binding': dict(EMPTY_BINDING)}


def ensure_sessiond():
    from mochi_paths import INSTALLED
    directory = private_directory()
    if INSTALLED:
        # A successful socket connection activates the service. No handwritten
        # start lock participates in the installed systemd path.
        current = status()
        if current.get('generation'):
            return
        if 'session.version_mismatch' in current['state'].get('service_error', ''):
            raise ConnectionError(current['state']['service_error'])
        try:
            ready = subprocess.run(['systemctl', '--user', 'start', 'mochi-sessiond.socket'],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, timeout=4).returncode == 0
        except (OSError, subprocess.TimeoutExpired):
            ready = False
        if ready:
            deadline = time.monotonic()+5
            while time.monotonic() < deadline:
                if status().get('generation'):
                    return
                time.sleep(.05)
            raise ConnectionError('Systemd session activation failed; inspect the user Journal.')
        # Compatibility fallback for installations without a user systemd bus.
        # Desktop installations always prefer socket activation above.
    with (directory/'sessiond-start.lock').open('a+b') as lock:
        os.chmod(directory/'sessiond-start.lock', 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX)
        current = status()
        if 'session.version_mismatch' in current['state'].get('service_error', ''):
            raise ConnectionError(current['state']['service_error'])
        if current['generation']:
            return
        process = subprocess.Popen(['/usr/bin/python3', '-B', str(Path(__file__).with_name('sessiond.py'))],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                   start_new_session=True, close_fds=True)
        deadline = time.monotonic()+3
        while time.monotonic() < deadline:
            if status()['generation']:
                return
            if process.poll() is not None:
                break
            time.sleep(.02)
        raise ConnectionError('Could not start mochi-sessiond.')


class Subscription:
    def __init__(self, callback, dispatch=lambda operation, *args: operation(*args), scope="chatgpt"):
        self.scope = scope
        self.callback, self.dispatch = callback, dispatch
        self.closed = threading.Event()
        self.connection = None
        self.thread = threading.Thread(target=self._run, name='mochi-session-events', daemon=True)
        self.thread.start()

    def _deliver(self, frame):
        if not self.closed.is_set():
            self.callback(frame)
        return False

    def _run(self):
        while not self.closed.is_set():
            try:
                with connect() as connection:
                    self.connection = connection
                    connection.settimeout(25)
                    connection.sendall(json.dumps({'version': VERSION, 'operation': 'subscribe', 'scope': self.scope}).encode()+b'\n')
                    with connection.makefile('rb') as stream:
                        while not self.closed.is_set():
                            raw = stream.readline(MAX_FRAME+1)
                            if not raw.endswith(b'\n') or len(raw) > MAX_FRAME:
                                raise ConnectionError('Session subscription ended.')
                            frame = json.loads(raw)
                            if frame.get('version') != VERSION or 'state' not in frame:
                                raise ConnectionError('Invalid session event.')
                            frame['_observed_at'] = time.monotonic()
                            self.dispatch(self._deliver, frame)
            except (OSError, ValueError):
                self.dispatch(self._deliver, {'generation': None, 'revision': 0, '_observed_at': time.monotonic(), 'state': dict(DISCONNECTED), 'binding': dict(EMPTY_BINDING)})
            finally:
                self.connection = None
            self.closed.wait(1)

    def close(self):
        self.closed.set()
        if self.connection:
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


class ReplyBindingAdapter:
    """Mailbox-shaped transport for BubbleHost; no locally-owned binding or tokens."""
    def __init__(self, changed, dispatch, scope="chatgpt", before_relaunch=lambda: None):
        self.before_relaunch = before_relaunch
        self.scope = scope
        self.frame = self._snapshot()
        self.changed = changed
        self.subscription = Subscription(self._changed, dispatch, scope)

    def _changed(self, frame):
        if frame.get('_observed_at', 0) < self.frame.get('_observed_at', 0):
            return
        value = self._snapshot()
        if value.get('generation'):
            self.last_generation = value['generation']
        self.frame = value
        self.changed()

    def _snapshot(self):
        try:
            frame = request('binding', scope=self.scope)
            frame['_observed_at'] = time.monotonic()
            return frame
        except (OSError, ValueError):
            current = status(self.scope)
            previous = getattr(self, 'last_generation', None) or getattr(self, 'frame', {}).get('generation')
            if (previous and current.get('generation') and previous != current['generation']
                    and inherited_authority().get('role_hint') == 'host'):
                # New daemon generations never accept old credentials. Re-enter
                # the fixed trusted launcher to obtain fresh inherited authority.
                self.before_relaunch()
                command = ['/usr/bin/python3', str(Path(__file__).with_name('session_launch.py')), 'host', sys.argv[1]]
                os.execv(command[0], command)
            return {**current, 'binding': dict(EMPTY_BINDING)}

    def refresh(self):
        self.frame = self._snapshot()

    @property
    def active(self):
        return self.frame['binding']['key']

    def snapshot(self):
        return self.frame['binding']

    def connection_status(self):
        return self.frame['state']

    def _call(self, operation, **arguments):
        try:
            result = request(operation, scope=self.scope, **arguments)
        except OSError as error:
            self.frame = self._snapshot()
            raise ValueError('Session disconnected; delivery may be uncertain. Reconnect the conversation card.') from error
        self.frame = self._snapshot()
        return result

    def create(self):
        return self._call('create')

    def attach(self, token, client):
        return self._call('attach', token=token, client=client)

    def poll(self, token, client):
        return self._call('poll', token=token, client=client)

    def acknowledge(self, token, client, identity, state):
        return self._call('acknowledge', token=token, client=client, reply_id=identity, state=state)

    def detach(self, token, client):
        return self._call('detach', token=token, client=client)

    def submit(self, key, text):
        return self._call('submit', key=key, text=text)['reply_id']

    def observed(self, key):
        self._call('observed', key=key)

    def reset(self):
        self._call('disconnect')

    def close(self):
        self.subscription.close()


if __name__ == '__main__':
    ensure_sessiond()
