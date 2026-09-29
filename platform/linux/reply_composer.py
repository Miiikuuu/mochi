"""Small, source-neutral reply control shared by speech and plugin panels."""
from gi.repository import Gdk, Gtk


class MultilineEntry(Gtk.TextView):
    """Entry-compatible text API; keep IME confirmation separate from sending."""
    def __init__(self, send, changed):
        super().__init__(wrap_mode=Gtk.WrapMode.WORD_CHAR)
        self.preedit = ''
        self.connect('preedit-changed', lambda _view, text: setattr(self, 'preedit', text))
        self.connect('key-press-event', self.key_pressed, send)
        self.get_buffer().connect('changed', changed)
        self.get_accessible().set_name('回复')
        self.set_tooltip_text('Enter 发送 · Shift+Enter 换行')

    def key_pressed(self, _view, event, send):
        if event.keyval in (Gdk.KEY_Return, Gdk.KEY_KP_Enter) and not event.state & Gdk.ModifierType.SHIFT_MASK:
            if self.preedit:return False
            # Give the input method first refusal, including a composition
            # that starts and commits in this event (without prior preedit).
            if self.im_context_filter_keypress(event):return True
            send()
            return True
        return False

    def get_text(self):
        buffer = self.get_buffer()
        return buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)

    def set_text(self, text):
        self.get_buffer().set_text(text)


class ReplyComposer(Gtk.Box):
    def __init__(self, submit, resized, multiline=False):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.submit, self.resized = submit, resized
        self.key, self.clear_id, self.spec = None, '', None
        self.submitted = None
        self.revision = 0
        self.set_no_show_all(True)
        self.get_style_context().add_class('mochi-reply')
        row = Gtk.Box(spacing=6)
        self.entry = MultilineEntry(self.send, self.changed) if multiline else Gtk.Entry(placeholder_text='回复…', max_length=4000)
        self.entry.set_hexpand(True)
        if multiline:
            self.entry.get_style_context().add_class('mochi-speech-text')
            self.scroll = Gtk.ScrolledWindow()
            self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
            self.scroll.set_size_request(-1, 58)
            self.scroll.add(self.entry)
            self.pack_start(self.scroll, True, True, 0)
            self.scroll.show_all()
        else:
            self.entry.connect('activate', lambda *_: self.send())
            self.entry.connect('changed', self.changed)
            row.pack_start(self.entry, True, True, 0)
        self.row = row
        self.button = Gtk.Button.new_from_icon_name('mail-send-symbolic', Gtk.IconSize.BUTTON)
        self.button.set_tooltip_text('发送回复')
        self.button.connect('clicked', lambda *_: self.send())
        row.pack_end(self.button, False, False, 0)
        self.pack_start(row, False, False, 0)
        self.error = Gtk.Label(xalign=0)
        self.error.set_line_wrap(True)
        self.error.set_max_width_chars(32)
        self.error.set_no_show_all(True)
        self.pack_start(self.error, False, False, 0)
        row.show_all()

    def sensitivity(self):
        text = self.entry.get_text()
        self.button.set_sensitive(bool(self.spec and self.spec.enabled and text.strip() and len(text.encode('utf-16-le'))//2 <= 4000))

    def changed(self, *_):
        self.revision += 1
        self.sensitivity()

    def render(self, spec):
        self.spec = spec
        if spec:
            new_channel = spec.key != self.key
            acknowledged = self.submitted and self.submitted == (spec.clear_id, self.revision)
            if new_channel or acknowledged:
                self.entry.set_text('')
                self.error.hide()
                self.submitted = None
            self.key, self.clear_id = spec.key, spec.clear_id
        self.set_visible(spec is not None)
        self.sensitivity()

    def send(self):
        if not self.spec or not self.button.get_sensitive():
            return
        try:
            revision = self.revision
            identity = self.submit(self.entry.get_text())
            self.submitted = (identity, revision)
            self.error.hide()
        except ValueError as error:
            self.error.set_text(str(error))
            self.error.show()
        self.resized()
