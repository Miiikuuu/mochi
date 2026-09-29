"""Single toolkit-free write, conflict and Undo boundary for Theme editing."""
from manager_state import AppError, error_from_exception
from manager_undo import UndoCommand, UndoExpired


class ThemeEditCoordinator:
    def __init__(self, controller, scheduler, *, refresh, error, clear_error,
                 conflict, show_undo, hide_undo, notice):
        self.controller, self.scheduler = controller, scheduler
        self.refresh, self.error, self.clear_error = refresh, error, clear_error
        self.conflict, self.show_undo, self.hide_undo, self.notice = conflict, show_undo, hide_undo, notice
        self.save_timer = self.undo_timer = self.undo_command = None
        self.disposed = False

    def present_error(self, failure):
        issue = failure if isinstance(failure, AppError) else error_from_exception(failure, 'theme')
        self.error(issue)
        if issue.code == 'theme.conflict':
            self.invalidate_undo()
            self.conflict()
        self.refresh(False)

    def edit(self, operation, *, scope=None, deferred=False):
        if self.disposed:
            return False
        if not deferred and not self.flush():
            self.refresh(True)
            return False
        before = self.controller.state.draft.snapshot()
        try:
            operation()
        except Exception as failure:
            self.present_error(failure)
            return False
        self.clear_error()
        if deferred:
            self.cancel_save()
            self.save_timer = self.scheduler.call_later(350, self._save_after_delay)
        else:
            self.invalidate_undo()
            after = self.controller.state.draft.snapshot()
            if scope and after != before:
                self.offer_undo(getattr(UndoCommand, scope)(before, after, self.controller.store.revision))
        self.refresh(not deferred)
        return True

    def cancel_save(self):
        if self.save_timer is not None:
            self.scheduler.cancel(self.save_timer)
            self.save_timer = None

    def _save_after_delay(self):
        self.save_timer = None
        self.flush()

    def flush(self):
        self.cancel_save()
        if self.disposed:
            return False
        if not self.controller.state.draft.needs_save:
            return True
        try:
            self.controller.save()
        except Exception as failure:
            self.present_error(failure)
            return False
        self.clear_error()
        self.refresh(False)
        return True

    def offer_undo(self, command):
        self.invalidate_undo()
        self.undo_command = command
        self.show_undo('Cast updated' if command.scope == 'cast' else 'Feature updated', lambda: self.undo(command))
        self.undo_timer = self.scheduler.call_later(6000, self.expire_undo)

    def invalidate_undo(self):
        self.undo_command = None
        if self.undo_timer is not None:
            self.scheduler.cancel(self.undo_timer)
            self.undo_timer = None
        self.hide_undo()

    def expire_undo(self):
        self.undo_timer = None
        self.undo_command = None
        self.hide_undo()

    def undo(self, command):
        if self.disposed:
            return False
        if command is not self.undo_command:
            self.notice('Undo expired; the current Theme was not changed.')
            return False
        self.invalidate_undo()
        try:
            self.controller.apply_undo(command)
        except UndoExpired:
            self.notice('Undo expired; the current Theme was not changed.')
            return False
        except Exception as failure:
            self.present_error(failure)
            return False
        self.refresh(True)
        return True

    def dispose(self):
        self.disposed = True
        self.cancel_save()
        self.invalidate_undo()
