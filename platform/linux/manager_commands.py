"""Serial command lane with coalesced status reads and explicit job kinds."""
from collections import deque
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from typing import Callable

from manager_state import AppError, CommandState, JobKind, error_from_exception


@dataclass
class Job:
    kind: JobKind
    operation: Callable
    completed: Callable
    started: Callable | None = None


class CommandRunner:
    def __init__(self, state: CommandState, dispatch: Callable):
        self.state = state
        self.dispatch = dispatch
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix='mochi-manager')
        self.queue = deque()
        self.active = None
        self.closed = False

    def submit(self, kind: JobKind, operation: Callable, completed: Callable, started: Callable | None = None) -> bool:
        if self.closed or self.state.closing:
            return False
        if kind == JobKind.STATUS and (self.active and self.active.kind == kind or any(job.kind == kind for job in self.queue)):
            return False
        self.queue.append(Job(kind, operation, completed, started))
        self.state.queued.append(kind)
        self._pump()
        return True

    def _pump(self):
        if self.active or not self.queue or self.closed:
            return
        job = self.active = self.queue.popleft()
        self.state.queued.pop(0)
        self.state.active = job.kind
        if job.started:
            job.started()

        def perform():
            try:
                result, error = job.operation(), None
            except Exception as failure:
                result, error = None, error_from_exception(failure, job.kind.value)
            self.dispatch(self._finish, job, result, error)

        self.executor.submit(perform)

    def _finish(self, job, result, error):
        if self.closed or self.active is not job:
            return False
        self.active = None
        self.state.active = None
        self.state.last_error_code = error.code if error else None
        try:
            job.completed(result, error)
        finally:
            self._pump()
        return False

    def mark_external(self, kind: JobKind, active: bool):
        if active:
            self.state.external.add(kind)
        else:
            self.state.external.discard(kind)

    def cancel_pending(self, kind=None):
        """Cancel only work that has not started; active runtime work must finish."""
        self.state.cancel_requested = True
        cancelled = [job for job in self.queue if kind is None or job.kind == kind]
        self.queue = deque(job for job in self.queue if job not in cancelled)
        self.state.queued[:] = [job.kind for job in self.queue]
        for job in cancelled:
            job.completed(None, AppError(job.kind.value+'.cancelled', 'Cancelled.'))
        return len(cancelled)

    def shutdown(self):
        self.closed = True
        self.cancel_pending()
        self.active = None
        self.state.active = None
        self.executor.shutdown(wait=False)
