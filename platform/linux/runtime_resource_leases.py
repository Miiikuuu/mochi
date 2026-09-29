"""Toolkit-free Runtime resource leases and the Apply/resource mutation gate."""
from contextlib import contextmanager
from dataclasses import asdict, dataclass
import fcntl
import json
import os
from pathlib import Path
import socket
import uuid
from desktop_ipc import socket_path
from resource_dependencies import file_lock


@dataclass(frozen=True)
class RuntimeDependencySnapshot:
    theme_id: str
    characters: tuple
    bubble: str
    plugins: tuple
    pid: int | None
    generation: str
    retained_bubbles: tuple = ()

    @classmethod
    def from_status(cls, status):
        theme = status.get('theme') or {}
        return cls(theme.get('id', 'current-desktop'), tuple(theme.get('characters', status.get('characters', []))),
                   status.get('bubble', theme.get('bubble', '')), tuple(status.get('plugins', theme.get('plugins', []))),
                   status.get('pid'), str(status.get('generation', status.get('pid', 'unidentified'))))

    def uses(self, kind, key):
        return key in (self.characters if kind == 'characters' else self.plugins if kind == 'plugins' else (self.bubble, *self.retained_bubbles))


class RuntimeResourceInUse(ValueError):
    def __init__(self, kind, key, snapshots):
        self.kind, self.key, self.snapshots = kind, key, snapshots
        super().__init__('Resource is used by running Theme(s): '+', '.join(s.theme_id for s in snapshots)+
                         '. Repair saved Themes, then Apply replacement or Stop Theme before removal.')


def probe_runtime():
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(2)
        try:
            client.connect(str(socket_path()))
        except (FileNotFoundError, ConnectionRefusedError):
            return None
        client.sendall(b'{"type":"theme_status"}\n')
        data = bytearray()
        while b'\n' not in data:
            part = client.recv(65536)
            if not part:
                raise ConnectionError('Runtime lease cannot be verified.')
            data.extend(part)
            if len(data) > 65536:
                raise ValueError('Runtime lease response is too large.')
        value = json.loads(data.split(b'\n', 1)[0])
        if not isinstance(value, dict) or value.get('error'):
            raise ConnectionError('Runtime lease cannot be verified.')
        return value


class RuntimeResourceLeases:
    def __init__(self, directory, probe=None):
        self.directory = Path(directory)
        self.root = self.directory/'runtime-leases'
        self.probe = probe or probe_runtime

    @contextmanager
    def mutation(self):
        self.directory.mkdir(parents=True, exist_ok=True)
        with file_lock(self.directory/'runtime-resources.lock'):
            yield

    def reserve(self, theme):
        """Inherited shared FD survives manager death, including spawn/record races."""
        from resource_catalog import atomic_write
        self.root.mkdir(mode=0o700, exist_ok=True)
        generation = uuid.uuid4().hex
        fd = os.open(self.root/(generation+'.lock'), os.O_CREAT | os.O_RDWR | os.O_CLOEXEC, 0o600)
        fcntl.flock(fd, fcntl.LOCK_SH)
        value = asdict(RuntimeDependencySnapshot.from_status({'theme': theme, 'generation': generation}))
        atomic_write(self.root/(generation+'.json'), value)
        return generation, fd

    def update(self, generation, status):
        from resource_catalog import atomic_write
        path = self.root/(generation+'.json')
        if path.exists():
            atomic_write(path, asdict(RuntimeDependencySnapshot.from_status({**status, 'generation': generation})))

    def prepare_transition(self, status, theme):
        """Persist the union before IPC: a manager crash cannot release new bytes."""
        from resource_catalog import atomic_write
        generation = status.get('generation')
        if not generation:
            return
        old = RuntimeDependencySnapshot.from_status(status)
        value = RuntimeDependencySnapshot(theme['id'], tuple(set(old.characters) | set(theme['characters'])),
            old.bubble, tuple(set(old.plugins) | set(theme['plugins'])), old.pid, generation,
            tuple({theme['bubble']}))
        atomic_write(self.root/(generation+'.json'), asdict(value))

    def snapshots(self):
        from resource_catalog import read_bounded
        snapshots = []
        with self.mutation():
            for path in sorted(self.root.glob('*.json')):
                lock = path.with_suffix('.lock')
                with lock.open('a+b') as stream:
                    try:
                        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError:
                        value = json.loads(read_bounded(path, 65536))
                        snapshots.append(RuntimeDependencySnapshot(**value))
                    else:
                        path.unlink(missing_ok=True)
                        lock.unlink(missing_ok=True)
            status = self.probe()
            if status:
                actual = RuntimeDependencySnapshot.from_status(status)
                # Legacy CLI sessions do not always expose a Theme. Protect all
                # registered characters if their identities cannot be established.
                if not actual.characters:
                    from resource_catalog import ResourceCatalog
                    actual = RuntimeDependencySnapshot(actual.theme_id,
                        tuple(ResourceCatalog(self.directory).read()['characters']), actual.bubble,
                        actual.plugins, actual.pid, actual.generation)
                if status.get('generation'):
                    self.update(status['generation'], status)
                    snapshots = [s for s in snapshots if s.generation != status['generation']]
                snapshots.append(actual)
        return tuple(snapshots)

    def assert_unused(self, kind, key):
        active = tuple(item for item in self.snapshots() if item.uses(kind, key))
        if active:
            raise RuntimeResourceInUse(kind, key, active)
