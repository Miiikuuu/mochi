"""Gio directory monitors and GLib timers used by native Manager windows."""
from pathlib import Path
import gi
from gi.repository import Gio, GLib


class GioConfigEventSource:
    def watch(self, paths, callback):
        expected = {Path(path) for path in paths}
        monitors = []
        for directory in {path.parent for path in expected}:
            monitor = Gio.File.new_for_path(str(directory)).monitor_directory(Gio.FileMonitorFlags.NONE, None)
            def changed(_monitor, file, other, _event):
                for candidate in (file, other):
                    if candidate is not None:
                        path = Path(candidate.get_path())
                        if path in expected:
                            callback(path)
            monitor.connect('changed', changed)
            monitors.append(monitor)
        def close():
            for monitor in monitors:
                monitor.cancel()
        return close

    def watch_packages(self, directories, callback):
        monitors = []
        for directory in directories:
            try:
                monitor = Gio.File.new_for_path(directory).monitor_directory(Gio.FileMonitorFlags.NONE, None)
            except GLib.Error:
                continue
            monitor.connect('changed', lambda _monitor, _file, _other, event:
                            callback() if event != Gio.FileMonitorEvent.CHANGES_DONE_HINT else None)
            monitors.append(monitor)
        return lambda: [monitor.cancel() for monitor in monitors]


class GlibScheduler:
    def call_later(self, milliseconds, callback):
        def fire():
            callback()
            return False
        return GLib.timeout_add(milliseconds, fire)

    @staticmethod
    def cancel(handle):
        GLib.source_remove(handle)
