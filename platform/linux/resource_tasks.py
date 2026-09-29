"""Toolkit-free resource task phases; reporting is scoped to a worker thread."""
from contextlib import contextmanager
from dataclasses import dataclass
import threading

_LOCAL = threading.local()
PHASES = ('queued', 'validating', 'committing', 'refreshing', 'completed', 'failed', 'cancelled')


@dataclass(frozen=True)
class ResourceTask:
    id: int
    action: str
    kind: str
    phase: str
    error_code: str | None = None


@contextmanager
def reporting(callback):
    old = getattr(_LOCAL, 'callback', None)
    _LOCAL.callback = callback
    try:
        yield
    finally:
        _LOCAL.callback = old


def report(phase):
    if phase not in PHASES:
        raise ValueError('Unknown resource task phase.')
    callback = getattr(_LOCAL, 'callback', None)
    if callback:
        callback(phase)
