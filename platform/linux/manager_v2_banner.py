"""Banner shell component."""
from manager_v2_common import Gtk, styled, text, clear
from manager_i18n import _


class Banner:
    def __init__(self, layout, callbacks):
        self.layout, self.callbacks = layout, callbacks
        gap = layout['spacing']
        self.config_banner = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=gap['sm']), 'config-banner')
        self.config_banner.set_visible(False)
        self.config_banner_label = text('', 'config-banner-text')
        self.config_banner_label.set_wrap(True)
        self.config_banner.append(self.config_banner_label)
        self.config_banner_actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=gap['sm'])
        self.config_banner.append(self.config_banner_actions)
        self.root = self.config_banner

    def show_banner(self, message, actions):
        self.config_banner_label.set_text(_(message))
        self._clear(self.config_banner_actions)
        for title, callback in actions:
            button = Gtk.Button(label=_(title))
            button.connect('clicked', lambda _button, operation=callback: operation())
            self.config_banner_actions.append(button)
        self.config_banner.set_visible(True)


    def hide_banner(self):
        self.config_banner.set_visible(False)


    _clear = staticmethod(clear)

    def responsive(self, compact):
        self.config_banner_actions.set_orientation(Gtk.Orientation.VERTICAL if compact else Gtk.Orientation.HORIZONTAL)
