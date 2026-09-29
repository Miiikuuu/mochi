"""Local Theme lifecycle service. The management view never owns pet rendering."""
from contextlib import contextmanager
from copy import deepcopy
import fcntl
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import uuid
from desktop_ipc import socket_path
from theme_store import DEFAULT, ROOT, atomic_json


class ThemeRuntime:
    def __init__(self, store, binary=None):
        self.store = store
        from mochi_paths import RENDERER
        self.binary = Path(binary or RENDERER).resolve()
        self.children = []

    def request(self, payload):
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(2)
            client.connect(str(socket_path()))
            client.sendall(json.dumps(payload).encode()+b'\n')
            data = bytearray()
            while b'\n' not in data:
                chunk = client.recv(65536)
                if not chunk:
                    raise ValueError('桌宠连接已关闭。')
                data.extend(chunk)
                if len(data) > 65536:
                    raise ValueError('桌宠返回的数据过长。')
            result = json.loads(data.split(b'\n', 1)[0])
            if not isinstance(result, dict) or result.get('error'):
                raise ValueError(result.get('error', '桌宠返回了无效状态。'))
            return result

    def status(self):
        self.children = [child for child in self.children if child.poll() is None]
        try:
            status = self.request({'type': 'theme_status'})
        except (FileNotFoundError, ConnectionRefusedError):
            return None
        pid = status['pid']
        # A rebuilt executable has a new inode; the existing, identified owner
        # can still be stopped/adopted until the user applies the new config.
        executable = Path(os.readlink(f'/proc/{pid}/exe').removesuffix(' (deleted)')).resolve() if type(pid) is int else None
        if executable != self.binary:
            raise ValueError('当前桌宠不是这个项目启动的实例。')
        if not status.get('theme'):
            args = Path(f'/proc/{pid}/cmdline').read_bytes().decode().strip('\0').split('\0')[1:]
            cwd = Path(f'/proc/{pid}/cwd').resolve()
            reverse = {str(Path(item['path']).resolve()): key for key, item in self.store.catalog.items()}
            keys = [reverse[str((cwd/arg).resolve())] for arg in args if str((cwd/arg).resolve()) in reverse]
            if not keys:
                raise ValueError('无法识别正在运行的角色，请从启动它的窗口关闭后重试。')
            theme = deepcopy(DEFAULT)
            theme.update(primary=keys[0], characters=keys, bubble=status['bubble'], plugins=status['plugins'])
            for option, field, cast in [('--size', 'size', int), ('--idle-walk-after', 'idle_after', lambda v: int(float(v)))]:
                if option in args:
                    theme[field] = cast(args[args.index(option)+1])
            # Adopt an existing CLI session only when its actual configuration
            # matches; do not pretend another saved Theme is already applied.
            for candidate in self.store.themes:
                if self.same_config(candidate, theme):
                    theme = candidate
                    break
            else:
                theme.update(id='current-desktop', name='当前桌面')
            status['theme'] = theme
        return status

    def disconnect_chatgpt(self):
        from session_client import request
        request('disconnect')
        return self.status()

    @staticmethod
    def same_config(left, right):
        fields = ('primary', 'characters', 'plugins', 'bubble', 'size', 'idle_after')
        return all(left.get(field) == right.get(field) for field in fields) and all(left.get(field, {}) == right.get(field, {}) for field in ('actions', 'plugin_settings'))

    @contextmanager
    def lock(self):
        path = socket_path().parent/'theme-control.lock'
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with path.open('a') as stream:
            os.chmod(path, 0o600)
            try:
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise ValueError('另一个窗口正在切换 Theme，请稍后重试。') from error
            try:
                yield
            finally:
                fcntl.flock(stream, fcntl.LOCK_UN)

    def _stop(self, status):
        if not status:
            return
        # Ask the identified owner to stop; never issue the global --stop
        # command after a race that might target a replacement session.
        self.request({'type': 'theme_stop', 'pid': status['pid']})
        deadline = time.monotonic()+8
        while time.monotonic() < deadline:
            if not Path(f"/proc/{status['pid']}").exists() and not socket_path().exists():
                return
            for child in self.children:
                child.poll()
            time.sleep(.03)
        raise ValueError('桌宠仍在关闭，请稍后重试。')

    def stop(self):
        with self.store.resources.leases.mutation(), self.lock():
            self._stop(self.status())
        return None

    def _launch(self, theme, positions, translation=None):
        keys, paths = self.store.paths(theme)
        run = self.store.directory/'runs'/uuid.uuid4().hex
        run.mkdir(parents=True, mode=0o700)
        env = {**os.environ, 'MOCHI_THEME_CONFIG': json.dumps(theme), 'GDK_BACKEND': 'x11'}
        from character_actions import runtime_policy
        env['MOCHI_ACTION_POLICY'] = json.dumps(runtime_policy(theme, keys, self.store.catalog))
        env.pop('MOCHI_THEME_TRANSLATION', None)
        if translation is not None:
            env['MOCHI_THEME_TRANSLATION'] = json.dumps(translation)
        generation, lease_fd = self.store.resources.leases.reserve(theme)
        env['MOCHI_RUNTIME_GENERATION'] = generation
        try:
            with (run/'desktop.log').open('w') as log:
                proc = subprocess.Popen([str(self.binary), *paths, '--size', str(theme['size']), '--idle-walk-after', str(theme['idle_after'])],
                                    cwd=ROOT, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True, pass_fds=(lease_fd,))
                self.store.resources.leases.update(generation, {'pid': proc.pid, 'theme': theme})
        finally:
            os.close(lease_fd)
        self.children.append(proc)
        deadline = time.monotonic()+15
        try:
            while time.monotonic() < deadline:
                if proc.poll() is not None:
                    raise ValueError('Theme 启动失败：'+(run/'desktop.log').read_text()[-800:])
                status = self.status()
                if status and status['pid'] == proc.pid:
                    restore = [{'window': role['window'], 'position': positions[key]} for key, role in zip(keys, status['roles']) if key in positions]
                    if restore:
                        self.request({'type': 'theme_positions', 'positions': restore})
                    status = self.status()
                    atomic_json(run/'session.json', status)
                    return status
                time.sleep(.05)
            raise ValueError('Theme 启动超时。')
        except BaseException:
            if proc.poll() is None:
                # Termination here only targets the child just created above.
                proc.terminate()
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
            raise

    def apply(self, theme, translation=None):
        with self.store.resources.leases.mutation():
            return self._apply(theme, translation)

    def _apply(self, theme, translation=None):
        theme = self.store.validate(theme)
        self.store.require_active_theme(theme)
        for key in theme['plugins']:
            self.store.plugins.require_approved(key, verify=True)
        if translation is not None:
            from manager_preferences import translation_options
            translation = translation_options(translation)
        _, paths = self.store.paths(theme)
        if not self.binary.is_file():
            raise ValueError('请先运行 scripts/run-spine-desktop.sh 完成桌宠构建。')
        if any(not (Path(path)/'manifest.toml').is_file() for path in paths):
            raise ValueError('角色素材已移动，请重新导入。')
        with self.lock():
            old = self.status()
            shape_fields = ('primary', 'characters', 'size', 'idle_after')
            if old and 'external_replies' in old.get('plugin_features', []) and (translation is None or 'translation' in old) and old['theme'].get('actions', {}) == theme.get('actions', {}) and old['theme']['id'] == theme['id'] and all(old['theme'][field] == theme[field] for field in shape_fields) and len(old['roles']) == len(theme['characters']) and all(role['alive'] for role in old['roles']):
                self.store.resources.leases.prepare_transition(old, theme)
                self.request({'type': 'theme_apply', 'theme': theme, 'translation': translation})
                current = self.status()
                if current and current.get('generation'):
                    self.store.resources.leases.update(current['generation'], current)
                return current
            positions = {}
            if old:
                keys, _ = self.store.paths(old['theme'])
                positions = {key: role['position'] for key, role in zip(keys, old['roles']) if role['alive']}
                self._stop(old)
            try:
                return self._launch(theme, positions, translation)
            except (OSError, ValueError) as error:
                if old:
                    try:
                        self._launch(old['theme'], positions, old.get('translation'))
                    except (OSError, ValueError) as recovery:
                        raise ValueError(f'{error}\n原 Theme 也未能恢复：{recovery}') from error
                    raise ValueError(f'{error}\n已恢复之前的 Theme。') from error
                raise
