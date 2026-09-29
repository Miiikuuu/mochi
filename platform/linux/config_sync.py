"""Toolkit-free, debounced change detection for Manager configuration files."""
from dataclasses import dataclass
import json
from pathlib import Path
from resource_catalog import read_bounded


@dataclass(frozen=True)
class ConfigChange:
    kind: str
    revision: int


class ConfigSyncService:
    """Observe durable revisions; event source and timer are injected by the host."""
    def __init__(self, themes_path, preferences_path, event_source, scheduler, changed, debounce_ms=180, resources_path=None):
        self.paths = {'themes': Path(themes_path), 'preferences': Path(preferences_path)}
        if resources_path is not None:
            self.paths['resources'] = Path(resources_path)
        self.event_source = event_source
        self.package_directories = ()
        self.stop_packages = lambda: None
        self.force_resources = False
        self.scheduler = scheduler
        self.changed = changed
        self.debounce_ms = debounce_ms
        self.revisions = {kind: self._revision(path) for kind, path in self.paths.items()}
        self.pending = set()
        self.timer = None
        self.closed = False
        self.stop_watching = event_source.watch(tuple(self.paths.values()), self.file_event)

    def observe_packages(self, directories):
        directories = tuple(directories)
        if self.closed or directories == self.package_directories:
            return
        self.stop_packages()
        self.package_directories = directories
        if hasattr(self.event_source, 'watch_packages'):
            self.stop_packages = self.event_source.watch_packages(directories, self.package_event)

    def package_event(self):
        if not self.closed and 'resources' in self.paths:
            self.force_resources = True
            self.file_event(self.paths['resources'])

    @staticmethod
    def _revision(path):
        if not path.exists():
            return 0
        value = json.loads(read_bounded(path, 4*1024*1024))
        revision = value.get('revision', 0) if isinstance(value, dict) else None
        if type(revision) is not int or revision < 0:
            raise ValueError(f'Invalid configuration revision in {path.name}.')
        return revision

    def file_event(self, path):
        if self.closed:
            return
        changed = Path(path) if path is not None else None
        for kind, watched in self.paths.items():
            if changed is None or changed == watched:
                self.pending.add(kind)
        if not self.pending:
            return
        if self.timer is not None:
            self.scheduler.cancel(self.timer)
        self.timer = self.scheduler.call_later(self.debounce_ms, self.flush)

    def flush(self):
        self.timer = None
        if self.closed:
            return
        pending, self.pending = self.pending, set()
        force_resources, self.force_resources = self.force_resources, False
        for kind in sorted(pending):
            try:
                revision = self._revision(self.paths[kind])
            except (OSError, ValueError):
                # A writer may still be replacing the file. The next monitor
                # event will retry without advancing our last good revision.
                continue
            if revision != self.revisions[kind] or kind == 'resources' and force_resources:
                self.revisions[kind] = revision
                self.changed(ConfigChange(kind, revision))

    def close(self):
        if self.closed:
            return
        self.closed = True
        if self.timer is not None:
            self.scheduler.cancel(self.timer)
            self.timer = None
        self.stop_packages()
        self.stop_watching()
