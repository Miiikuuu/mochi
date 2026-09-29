"""Manager preferences; no persistence or platform services in this page."""
from manager_v2_common import Page, Gtk, text
from manager_i18n import _


class SettingsPage(Page):
    def __init__(self, *args):
        super().__init__(*args)
        self.root.set_spacing(self.layout['spacing']['md'])
        self.root.append(text('Settings', 'page-title'))
        self.controls = {}
        self.guard = False
        self.choices = {
            'appearance': [('system', 'System'), ('light', 'Light'), ('dark', 'Dark')],
            'close': [('background', 'Keep Theme running'), ('quit', 'Stop Theme and close')],
            'language': [('system', 'System'), ('en', 'English'), ('zh_CN', '简体中文')]}
        for key, title in [('appearance', 'Appearance'), ('close', 'Close behavior'), ('language', 'Language')]:
            self.root.append(text(title, 'field-label'))
            control = Gtk.DropDown.new_from_strings([_(label) for _key, label in self.choices[key]])
            control.update_property([Gtk.AccessibleProperty.LABEL], [_(title)])
            control.connect('notify::selected', lambda widget, _param, name=key:
                            self.callbacks['setting'](name, self.choices[name][widget.get_selected()][0]) if not self.guard else None)
            self.controls[key] = control
            self.root.append(control)
        for key, title in [('start_on_login', 'Open Manager on login'), ('reduced_motion', 'Reduce Manager motion')]:
            row = Gtk.Box(spacing=self.layout['spacing']['md'])
            label = text(title, 'field-label')
            label.set_hexpand(True)
            row.append(label)
            control = Gtk.Switch()
            control.update_property([Gtk.AccessibleProperty.LABEL], [_(title)])
            control.connect('notify::active', lambda widget, _param, name=key:
                            self.callbacks['setting'](name, widget.get_active()) if not self.guard else None)
            row.append(control)
            self.controls[key] = control
            self.root.append(row)
        self.root.append(text('Log location', 'field-label'))
        self.log_location = text(self.callbacks['settings_model']()['log_location'], 'section-hint')
        self.log_location.set_selectable(True)
        self.root.append(self.log_location)
        reset = Gtk.Button(label=_('Reset Manager preferences'))
        reset.add_css_class('destructive-action')
        reset.connect('clicked', lambda _button: self.callbacks['settings_reset']())
        reset.update_property([Gtk.AccessibleProperty.LABEL], [_('Reset Manager preferences')])
        self.controls['reset'] = reset
        self.root.append(reset)
        self.update_settings()

    def update_settings(self):
        self.guard = True
        try:
            for key, choices in self.choices.items():
                self.controls[key].set_selected(next(i for i, (value, _) in enumerate(choices) if value == self.state.settings[key]))
            for key in ('start_on_login', 'reduced_motion'):
                self.controls[key].set_active(self.state.settings[key])
        finally:
            self.guard = False

    def set_width(self, width):
        available = width - self.layout['railWidth'] - self.layout['spacing']['lg']*4
        limit = self.layout['settingsMaxWidth']
        self.root.set_halign(Gtk.Align.START if available > limit else Gtk.Align.FILL)
        self.root.set_size_request(limit if available > limit else -1, -1)
