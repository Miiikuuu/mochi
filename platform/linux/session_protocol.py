"""Mochi session protocol v1: private, same-UID Unix transport."""
import os
from pathlib import Path
import socket
import stat
import struct

VERSION = 2
from mochi_version import VERSION as BUILD
CAPABILITIES = ("role-authority", "sanitized-state", "revision-events", "boottime", "per-user-binding")
MAX_FRAME = 65536


def socket_path():
    base = Path(os.environ.get('XDG_RUNTIME_DIR', f'/tmp/mochi-{os.getuid()}'))
    return base/'mochi'/'session-v1.sock'


def private_directory():
    path = socket_path().parent
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid():
        raise PermissionError('Unsafe session directory.')
    os.chmod(path, 0o700)
    return path


def require_peer(connection, uid=None):
    _pid, peer_uid, _gid = struct.unpack('3i', connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
    if peer_uid != (os.getuid() if uid is None else uid):
        raise PermissionError('Session peer UID does not match.')


def connect():
    path = socket_path()
    info = path.lstat()
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600:
        raise PermissionError('Unsafe session socket.')
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        connection.settimeout(1)
        connection.connect(str(path))
        require_peer(connection)
        return connection
    except BaseException:
        connection.close()
        raise
