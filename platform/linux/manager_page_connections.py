"""Native Manager connections page."""
from pathlib import Path
from gi.repository import Gdk, GdkPixbuf, GLib, Gtk, Pango
from manager_i18n import _
from manager_widgets import NAMES, box, combo, css, field, icon, icon_button, label, portrait

from mochi_paths import DATA as ROOT
CATEGORIES = {'characters': 'Characters', 'assets': 'Assets', 'plugins': 'Plugins'}


class ConnectionsPage:
    def render_connections(self, library=False):
        for key in self.plugin_keys():
            self.content.pack_start(self.plugin_card(key, library=library), False, False, 0)

    def plugin_keys(self):
        return list(dict.fromkeys([*self.state.plugins, *(self.theme['plugins'] if self.area != 'library' else [])]))


    def plugin_card(self, key, library=False):
        button = css(Gtk.Button(), 'plugin-card')
        row = box(spacing=16)
        name = self.plugin_name(key)
        spec = self.state.plugins.get(key, {})
        picture = icon('plug', 22)
        if spec.get('icon'):
            from gi.repository import Gio
            picture = Gtk.Image.new_from_gicon(Gio.FileIcon.new(Gio.File.new_for_path(str(spec['root']/spec['icon']))), Gtk.IconSize.MENU)
            picture.set_pixel_size(22)
        row.pack_start(css(picture, 'plugin-icon'), False, False, 0)
        row.pack_start(label(name, expand=True), True, True, 0)
        if not library:
            enabled = key in self.theme['plugins']
            row.pack_start(css(icon('circle-check' if enabled else 'circle', 16), 'muted'), False, False, 0)
            button.get_accessible().set_name(name+' settings, '+('enabled' if enabled else 'disabled'))
        else:
            button.get_accessible().set_name('Open '+name)
        row.pack_end(icon('chevron', 16), False, False, 0)
        button.add(row)
        button.connect('clicked', lambda *_: self.open_detail(key))
        self.register('plugin:'+key, button)
        return button


    def plugin_detail(self):
        self.heading(self.plugin_name(self.detail), True)
        spec = self.state.plugins.get(self.detail)
        if self.area == 'library':
            description = label(spec['description'] if spec else 'Plugin unavailable.', 'muted')
            description.set_ellipsize(Pango.EllipsizeMode.NONE)
            description.set_line_wrap(True)
            description.set_max_width_chars(45)
            self.content.pack_start(description, False, False, 0)
            self.content.pack_start(label('v'+spec['version'], 'muted'), False, False, 8)
            self.content.pack_start(self.usage(lambda theme: self.detail in theme['plugins'], 'plugins'), False, False, 24)
            return
        key = self.detail
        enabled = self.register('plugin_enabled', Gtk.Switch(active=key in self.theme['plugins']))
        if key == 'translation':
            self.register('translation', enabled)  # Existing native control identifier.
        enabled.connect('notify::active', lambda widget, _, key=key: self.plugin_changed(widget, key))
        self.content.pack_start(field('Enabled', enabled), False, False, 0)
        if not spec:
            self.content.pack_start(label('Plugin unavailable. Import its package to use it.', 'muted'), False, False, 0)
            return
        if spec['runtime'] != 'builtin':
            options = self.controller.plugin_settings(spec, self.theme.get('plugin_settings', {}).get(key))
            for setting in spec['settings']:
                identity, kind = setting['id'], setting['type']
                value = options[identity]
                if kind == 'boolean':
                    control = Gtk.Switch(active=value)
                    control.connect('notify::active', lambda w, _, identity=identity: self.plugin_setting_changed(key, identity, w.get_active()))
                elif kind == 'choice':
                    control = combo(setting['choices'], value)
                    control.connect('changed', lambda w, identity=identity: self.plugin_setting_changed(key, identity, w.get_active_id()))
                elif kind == 'integer':
                    control = Gtk.SpinButton.new_with_range(setting.get('min', -1000000), setting.get('max', 1000000), 1)
                    control.set_value(value)
                    control.connect('value-changed', lambda w, identity=identity: self.plugin_setting_changed(key, identity, w.get_value_as_int()))
                else:
                    control = Gtk.Entry(text=value)
                    control.set_max_length(setting.get('max_length', 500))
                    control.set_width_chars(18)
                    control.connect('changed', lambda w, identity=identity: self.plugin_setting_changed(key, identity, w.get_text()))
                self.register('plugin_setting:'+identity, control)
                self.content.pack_start(field(setting['label'], control), False, False, 0)
            return
        options = self.controller.translation(self.theme['id'])
        languages = {'zh-CN': 'Chinese (Simplified)', 'en': 'English', 'ja': 'Japanese', 'ko': 'Korean', 'fr': 'French', 'de': 'German', 'es': 'Spanish'}
        target = self.register('language', combo(languages, options['target']))
        target.connect('changed', lambda w: self.translation_changed(target=w.get_active_id()))
        self.content.pack_start(field('Target language', target), False, False, 0)
        row = box(spacing=8)
        model = self.register('model', Gtk.ComboBoxText.new_with_entry())
        for key, item in self.model_catalog.items():
            model.append(key, key)
        if options['model'] not in self.model_catalog:
            model.append(options['model'], options['model'])
        model.set_active_id(options['model'])
        entry = model.get_child()
        entry.set_width_chars(16)
        entry.set_max_width_chars(20)
        entry.set_max_length(128)
        entry.get_accessible().set_name('Model ID')
        model.connect('changed', lambda w: self.translation_changed(model=w.get_active_id()) if w.get_active_id() else None)
        entry.connect('activate', lambda w: self.translation_changed(model=w.get_text()))
        entry.connect('focus-out-event', lambda w, _: (self.translation_changed(model=w.get_text()), False)[1])
        row.pack_start(model, False, False, 0)
        refresh = self.action('models_refresh', 'refresh', 'Refresh available Codex models', self.refresh_models)
        refresh.set_sensitive(self.catalog_job is None)
        row.pack_start(refresh, False, False, 0)
        self.content.pack_start(field('Model', row), False, False, 0)
        fast = self.register('fast', Gtk.Switch(active=options['fast']))
        fast.set_tooltip_text(_('Faster responses may increase usage.'))
        fast.connect('notify::active', lambda w, _: self.translation_changed(fast=w.get_active()))
        self.content.pack_start(field('Fast', fast, 'bolt'), False, False, 0)
        self.sync_fast()
