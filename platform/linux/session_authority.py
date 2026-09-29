"""In-memory authority, issued only to daemon-launched trusted entrypoints.

The environment carries an FD number, never a credential. The sealed memfd is
inherited by the selected process only; subprocesses must use close_fds=True.
"""
import fcntl
import ctypes
import json
import os
import secrets
import struct

ROLES = {
    'card': frozenset(('create', 'attach', 'poll', 'acknowledge', 'detach')),
    'manager': frozenset(('disconnect',)),
    'host': frozenset(('binding', 'submit', 'observed')),
}
PUBLIC = frozenset(('status', 'subscribe', 'diagnostics', 'hello'))
_cached = None


def sealed_authority(value):
    fd = os.memfd_create('mochi-session-authority', os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    os.write(fd, json.dumps(value).encode())
    os.lseek(fd, 0, os.SEEK_SET)
    fcntl.fcntl(fd, fcntl.F_ADD_SEALS, fcntl.F_SEAL_WRITE | fcntl.F_SEAL_GROW | fcntl.F_SEAL_SHRINK | fcntl.F_SEAL_SEAL)
    return fd


def inherited_authority():
    global _cached
    if _cached is None:
        _cached = {}
        if ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) != 0:
            raise PermissionError('Cannot protect session authority memory.')
        raw = os.environ.pop('MOCHI_SESSION_FD', None)
        if raw:
            fd = int(raw)
            try:
                if not fcntl.fcntl(fd, fcntl.F_GET_SEALS) & fcntl.F_SEAL_WRITE:
                    raise PermissionError('Unsealed session authority.')
                _cached = json.loads(os.pread(fd, 4096, 0))
            finally:
                os.close(fd)
    return _cached


def peer_pid(connection):
    return struct.unpack('3i', connection.getsockopt(1, 17, 12))[0]


class AuthorityRegistry:
    def __init__(self):
        self.grants = {}

    def issue(self, role, scope='chatgpt', pid=None):
        if role not in ROLES and role != 'test':
            raise ValueError('Unknown authority role.')
        secret = secrets.token_hex(32)
        self.grants[secret] = (role, scope, pid)
        return {'capability': secret, 'scope': scope, 'role_hint': role}

    def authorize(self, request, pid):
        operation = request.get('operation')
        if operation in PUBLIC:
            if request.get('scope', 'chatgpt') != 'chatgpt':
                raise PermissionError('Session scope denied.')
            return 'chatgpt'
        grant = self.grants.get(request.get('capability'))
        if not grant:
            raise PermissionError('Session capability required.')
        role, scope, owner = grant
        if owner is not None and owner != pid:
            raise PermissionError('Session capability belongs to another process.')
        if role != 'test' and operation not in ROLES[role]:
            raise PermissionError('Session role denied.')
        if request.get('scope', scope) != scope:
            raise PermissionError('Session scope denied.')
        return scope
