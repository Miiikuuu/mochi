"""Native Manager settings page."""
from pathlib import Path
from gi.repository import Gdk, GdkPixbuf, GLib, Gtk, Pango
from manager_widgets import NAMES, box, combo, css, field, icon, icon_button, label, portrait

from mochi_paths import DATA as ROOT
CATEGORIES = {'characters': 'Characters', 'assets': 'Assets', 'plugins': 'Plugins'}


class SettingsPage:
    def render_settings(self):
        self.settings_page()

    def settings_page(self, theme=False):
        self.heading('Theme settings' if theme else 'Settings', theme)
        if theme:
            name = self.register('name', Gtk.Entry(text=self.theme['name']))
            name.set_width_chars(18)
            name.set_max_length(60)
            name.connect('changed', self.name_typed)
            name.connect('activate', self.name_changed)
            name.connect('focus-out-event', lambda w, _: (self.name_changed(w), False)[1])
            self.content.pack_start(field('Name', name), False, False, 0)
            size = self.register('size', Gtk.SpinButton.new_with_range(128, 512, 16))
            size.set_value(self.theme['size'])
            size.connect('value-changed', lambda w: self.edit_theme(size=w.get_value_as_int()))
            self.content.pack_start(field('Size', size), False, False, 0)
            idle = self.register('idle', Gtk.Switch(active=self.theme['idle_after'] != 0))
            idle.connect('notify::active', lambda w, _: self.edit_theme(idle_after=(self.theme['idle_after'] or 30) if w.get_active() else 0))
            self.content.pack_start(field('Idle behavior', idle), False, False, 0)
            self.heading_actions.pack_start(self.action('export', 'export', 'Export theme configuration', lambda *_: self.export_theme()), False, False, 0)
        else:
            appearance = self.register('appearance', combo({'system': 'System', 'light': 'Light', 'dark': 'Dark'}, self.state.settings['appearance']))
            appearance.connect('changed', lambda w: self.app_changed(appearance=w.get_active_id()))
            self.content.pack_start(field('Appearance', appearance, 'sun'), False, False, 0)
            close = self.register('on_close', combo({'background': 'Keep running', 'quit': 'Stop and quit'}, self.state.settings['close']))
            close.connect('changed', lambda w: self.app_changed(close=w.get_active_id()))
            self.content.pack_start(field('On close', close), False, False, 0)
