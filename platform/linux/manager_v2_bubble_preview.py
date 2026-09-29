"""Two-sided static message preview and its short-lived animations."""
from manager_v2_common import GLib, Gtk, styled, text, clear
from manager_i18n import _


class BubblePreview:
    def __init__(self, layout):
        self.layout = layout
        self.loading_timer = None
        self.message_fade_timer = None
        self.bubble_message_count = 2
        self.disposed = False
        stage = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['md']), 'bubble-stage')
        self.bubble_stage = stage
        stage.append(text('Preview Stage', 'section-title'))
        stage.append(text('Sample messages only; this does not start the desktop pet.', 'section-hint'))
        controls = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['sm'])
        controls.append(text('Messages', 'field-label'))
        self.bubble_count_buttons = {}
        for count in (1, 2, 3):
            button = Gtk.Button(label=str(count))
            button.update_property([Gtk.AccessibleProperty.LABEL], [_('%d sample messages') % count])
            button.connect('clicked', lambda _button, amount=count: self.set_bubble_message_count(amount))
            controls.append(button)
            self.bubble_count_buttons[count] = button
        stage.append(controls)
        self.bubble_stack = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['sm']), 'bubble-stack')
        stage.append(self.bubble_stack)
        self._render_bubble_messages(False)
        composer = styled(Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL), 'material-user')
        composer.set_halign(Gtk.Align.END)
        composer.append(text('Your reply…', 'bubble-copy'))
        stage.append(composer)
        loading = styled(Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['sm']), 'bubble-loading')
        self.loading_flower = text('✿', 'loading-flower')
        loading.append(self.loading_flower)
        loading.append(text('Thinking…', 'section-hint'))
        stage.append(loading)
        if Gtk.Settings.get_default().get_property('gtk-enable-animations'):
            self.loading_timer = GLib.timeout_add(360, self._pulse_loading)
        self.root = stage

    def _pulse_loading(self):
        if not self.loading_flower.get_parent():
            self.loading_timer = None
            return False
        self.loading_flower.set_opacity(.45 if self.loading_flower.get_opacity() > .7 else 1)
        return True


    def _stop_loading(self):
        if self.loading_timer is not None:
            GLib.source_remove(self.loading_timer)
            self.loading_timer = None
        if self.message_fade_timer is not None:
            GLib.source_remove(self.message_fade_timer)
            self.message_fade_timer = None


    def set_bubble_message_count(self, count):
        if count not in (1, 2, 3) or count == self.bubble_message_count:
            return
        self.bubble_message_count = count
        self._render_bubble_messages(True)


    def _render_bubble_messages(self, animate):
        animate = animate and Gtk.Settings.get_default().get_property('gtk-enable-animations')
        self._clear(self.bubble_stack)
        samples = (('assistant', 'A little thought from Mochi.'),
                   ('user', 'Can you tell me more?'),
                   ('assistant', 'Of course — here is the next note.'))[-self.bubble_message_count:]
        self.bubble_messages = []
        for speaker, content in samples:
            card = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL),
                          'material-user' if speaker == 'user' else 'material-speech')
            card.set_halign(Gtk.Align.END if speaker == 'user' else Gtk.Align.START)
            card.append(text(content, 'bubble-copy'))
            self.bubble_stack.append(card)
            self.bubble_messages.append(card)
        targets = [max(.36, 1 - .32 * (len(samples) - index - 1)) for index in range(len(samples))]
        for button in self.bubble_count_buttons.values():
            button.remove_css_class('active')
        self.bubble_count_buttons[self.bubble_message_count].add_css_class('active')
        if not animate:
            for card, target in zip(self.bubble_messages, targets):
                card.set_opacity(target)
            return
        if self.message_fade_timer is not None:
            GLib.source_remove(self.message_fade_timer)
        frame = 0
        def fade():
            nonlocal frame
            frame += 1
            for card, target in zip(self.bubble_messages, targets):
                card.set_opacity(target * min(1, frame / 9))
            if frame >= 9:
                self.message_fade_timer = None
                return False
            return True
        self.message_fade_timer = GLib.timeout_add(25, fade)


    _clear = staticmethod(clear)

    def dispose(self):
        self.disposed = True
        self._stop_loading()

    detach = dispose
