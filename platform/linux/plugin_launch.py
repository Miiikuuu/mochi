"""Process launch policy: immutable package, writable per-plugin state.

Landlock restricts filesystem writes (not network or process permissions). This
is NOT a complete plugin sandbox. Restrictions survive exec and child processes.
An optional Bubblewrap plan exposes explicit read-only/read-write mounts.
"""
from dataclasses import dataclass
import ctypes
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import subprocess
from functools import lru_cache


@dataclass(frozen=True)
class PluginLaunchPlan:
    command: tuple
    cwd: str
    environment: dict
    package_bind_ro: str
    state_bind_rw: str


def state_directory(key):
    if not re.fullmatch(r'[a-z][a-z0-9_-]{0,63}', key):
        raise ValueError('Invalid plugin ID.')
    root = Path(os.environ.get('XDG_STATE_HOME', str(Path.home()/'.local/state')))
    directory = root/'mochi/plugins'/key
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    if directory.is_symlink() or directory.stat().st_uid != os.getuid():
        raise ValueError('Plugin state must belong to the current user.')
    directory.chmod(0o700)
    return directory.resolve()


@lru_cache(maxsize=1)
def session_isolation_available():
    executable = shutil.which('bwrap')
    if not executable:
        return False
    try:
        return subprocess.run([executable, '--unshare-user', '--unshare-pid', '--unshare-net', '--ro-bind', '/', '/',
                               '--proc', '/proc', '--', '/bin/true'],
                              stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=3).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def launch_plan(spec, config_directory, *, bubblewrap=None):
    if bubblewrap is None:
        bubblewrap = not spec.get('builtin', False) and session_isolation_available()
    package = Path(spec['root']).resolve()
    state = state_directory(spec['id'])
    if state.is_relative_to(package) or package.is_relative_to(state):
        raise ValueError('Package and State must be separate.')
    environment = dict(os.environ, MOCHI_PLUGIN_STATE_DIR=str(state),
                       PYTHONDONTWRITEBYTECODE='1', TMPDIR=str(state/'tmp'),
                       XDG_CACHE_HOME=str(state/'cache'))
    if not spec.get('builtin'):
        environment = {key: value for key, value in environment.items() if key in (
            'PATH', 'LANG', 'LANGUAGE', 'LC_ALL', 'LC_MESSAGES', 'MOCHI_PLUGIN_STATE_DIR',
            'PYTHONDONTWRITEBYTECODE', 'TMPDIR', 'XDG_CACHE_HOME', 'XDG_STATE_HOME', 'XDG_RUNTIME_DIR')}
        environment['PYTHONNOUSERSITE'] = '1'
    environment.pop('MOCHI_SESSION_FD', None)
    for key in ('PYTHONPATH', 'PYTHONHOME', 'NODE_OPTIONS', 'LD_PRELOAD', 'LD_LIBRARY_PATH'):
        environment.pop(key, None)
    (state/'tmp').mkdir(exist_ok=True)
    (state/'cache').mkdir(exist_ok=True)
    command = (sys.executable, '-B', str(Path(__file__).resolve()), str(config_directory), spec['id'])
    if bubblewrap:
        command = (*command, '--bubblewrap')
    return PluginLaunchPlan(command, str(state), environment, str(package), str(state))


def restrict_writes(state, read_roots=None):
    """Fail closed if kernel write confinement (including truncate) is absent."""
    if platform.machine() not in ('x86_64', 'aarch64', 'riscv64'):
        raise OSError('Plugin write confinement is unavailable on this architecture.')
    libc = ctypes.CDLL(None, use_errno=True)
    def call(number, *args):
        result = libc.syscall(ctypes.c_long(number), *args)
        if result < 0:
            raise OSError(ctypes.get_errno(), 'Plugin filesystem write confinement failed')
        return result
    abi = call(444, 0, 0, 1)
    if abi < 3:
        raise OSError('Plugin write confinement requires Landlock ABI 3 or newer.')
    # WRITE_FILE, REMOVE_*, MAKE_*, REFER, TRUNCATE. Reads are unrestricted.
    rights = (1 << 1) | sum(1 << bit for bit in range(4, 15))
    if abi >= 5:
        rights |= 1 << 15  # IOCTL_DEV
    read_rights = (1 << 0) | (1 << 2) | (1 << 3)
    if read_roots is not None:
        rights |= read_rights
    ruleset = ctypes.c_uint64(rights)
    fd = call(444, ctypes.byref(ruleset), ctypes.sizeof(ruleset), 0)
    class Beneath(ctypes.Structure):
        _pack_ = 1
        _fields_ = [('rights', ctypes.c_uint64), ('fd', ctypes.c_int32)]
    try:
        paths = [(state, rights), ('/dev/null', (1 << 1) | (1 << 14))]
        if read_roots is not None:
            paths.extend((path, read_rights) for path in read_roots if Path(path).is_dir())
        for path, allowed in paths:
            parent = os.open(path, os.O_PATH | os.O_CLOEXEC)
            try:
                rule = Beneath(allowed, parent)
                call(445, fd, 1, ctypes.byref(rule), 0)
            finally:
                os.close(parent)
        if libc.prctl(38, 1, 0, 0, 0) != 0:
            raise OSError(ctypes.get_errno(), 'Cannot set no_new_privs')
        call(446, fd, 0)
    finally:
        os.close(fd)


def main():
    from plugin_catalog import PluginCatalog
    catalog = PluginCatalog(sys.argv[1])
    key = sys.argv[2]
    catalog.require_approved(key)
    spec = catalog.items[key]
    plan = launch_plan(spec, sys.argv[1])
    if not spec.get('builtin') and not session_isolation_available():
        raise ValueError('Session isolation unavailable: third-party execution is disabled.')
    executable = shutil.which(spec['runtime'])
    if not executable:
        raise ValueError('Plugin runtime is unavailable.')
    os.chdir(plan.cwd)
    entry = str(Path(spec['root'])/spec['entry'])
    command = [sys.executable, str(Path(__file__).with_name('plugin_exec.py')), plan.state_bind_rw, executable, entry, 'bundled' if spec.get('builtin') else 'local']
    if not spec.get('builtin'):
        runtime = str(Path(os.environ.get('XDG_RUNTIME_DIR', f'/tmp/mochi-{os.getuid()}')))
        command = ['bwrap', '--die-with-parent', '--unshare-user', '--unshare-pid', '--unshare-net',
                   '--ro-bind', '/', '/', '--proc', '/proc', '--tmpfs', runtime,
                   '--ro-bind', plan.package_bind_ro, plan.package_bind_ro,
                   '--bind', plan.state_bind_rw, plan.state_bind_rw, '--', *command]
        command[0] = shutil.which('bwrap') or '/usr/bin/bwrap'
    os.execve(command[0], command, plan.environment)


if __name__ == '__main__':
    main()
