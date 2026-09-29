"""Undo shell component."""
from manager_v2_common import GLib, Gtk, Pango, styled, text
from manager_i18n import _


class Undo:
    def __init__(self, layout, callbacks):
        self.layout, self.callbacks = layout, callbacks
        gap = layout['spacing']
        self.undo_bar = styled(Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=gap['md']), 'undo-bar')
        self.undo_bar.set_visible(False)
        self.undo_text = text('', 'undo-text')
        self.undo_text.set_hexpand(True)
        self.undo_text.set_wrap(True)
        self.undo_text.set_ellipsize(Pango.EllipsizeMode.END)
        self.undo_bar.append(self.undo_text)
        self.undo_button = Gtk.Button(label=_('Undo'))
        self.undo_bar.append(self.undo_button)
        self.root = self.undo_bar

    def show_undo(self, message, callback):
        if getattr(self, 'notice_timer', None) is not None:
            GLib.source_remove(self.notice_timer)
            self.notice_timer = None
        self.undo_text.set_text(_(message))
        if hasattr(self, '_undo_handler'):
            self.undo_button.disconnect(self._undo_handler)
        self._undo_handler = self.undo_button.connect('clicked', lambda *_: callback())
        self.undo_button.set_visible(True)
        self.undo_bar.set_visible(True)


    def show_notice(self, message):
        self.undo_text.set_text(_(message))
        self.undo_button.set_visible(False)
        self.undo_bar.set_visible(True)
        if getattr(self, 'notice_timer', None) is not None:
            GLib.source_remove(self.notice_timer)
        self.notice_timer = GLib.timeout_add_seconds(4, self._expire_notice)


    def _expire_notice(self):
        self.notice_timer = None
        self.undo_bar.set_visible(False)
        return False


    def hide_undo(self):
        self.undo_bar.set_visible(False)


    def dispose(self):
        if getattr(self, 'notice_timer', None) is not None:
            GLib.source_remove(self.notice_timer)
            self.notice_timer = None
        if hasattr(self, '_undo_handler'):
            self.undo_button.disconnect(self._undo_handler)
            del self._undo_handler

    detach = dispose
