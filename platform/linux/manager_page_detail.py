"""Native Manager detail page."""
from pathlib import Path
from gi.repository import Gdk, GdkPixbuf, GLib, Gtk, Pango
from manager_i18n import _
from manager_widgets import NAMES, box, combo, css, field, icon, icon_button, label, portrait

from mochi_paths import DATA as ROOT
CATEGORIES = {'characters': 'Characters', 'assets': 'Assets', 'plugins': 'Plugins'}


class DetailPage:
    def render_detail(self):
        tab = self.library_tab if self.area == 'library' else self.tab
        {'characters': self.character_detail, 'assets': self.bubble_detail, 'plugins': self.plugin_detail}[tab]()

    def character_detail(self):
        from character_actions import actions, settings, TRIGGERS
        key = self.detail
        records = actions(self.state.catalog[key])
        selected = self.selected_actions.get(key)
        record = next((a for a in records if a['id'] == selected), records[0] if records else None)
        if record:
            self.selected_actions[key] = record['id']
        self.heading(self.character_name(key), True)
        self.detail_box = box(spacing=36)
        left = box(True, 0)
        left.set_size_request(256, -1)
        left.set_valign(Gtk.Align.START)
        self.detail_box.pack_start(left, False, False, 0)
        self.detail_art = css(box(True, 0), 'large-preview')
        self.detail_art.set_size_request(256, 280)
        self.action_picture = portrait(key, self.state.catalog[key], 188, 210)
        self.action_picture.set_size_request(256, 256)
        self.detail_art.pack_start(self.action_picture, True, False, 0)
        if record:
            toolbar = box(spacing=8)
            toolbar.set_halign(Gtk.Align.CENTER)
            toolbar.set_margin_bottom(12)
            toolbar.pack_start(self.action('action_play', 'play', 'Play action preview', lambda *_: self.preview_action()), False, False, 0)
            toolbar.pack_start(self.action('action_stop', 'stop', 'Stop action preview', lambda *_: self.stop_action_preview()), False, False, 0)
            self.page.controls['action_stop'].set_sensitive(False)
            self.detail_art.pack_start(toolbar, False, False, 0)
        left.pack_start(self.detail_art, False, False, 0)
        fields = box(True, 0)
        if self.area != 'library':
            primary = key == self.theme['primary']
            enabled = self.register('enabled', Gtk.Switch(active=key in self.theme['characters']))
            enabled.set_sensitive(not primary and (key in self.theme['characters'] or len(self.theme['characters']) < 4))
            enabled.set_tooltip_text(_('Choose another primary character before disabling this one.') if primary else _('Enable this character in the theme'))
            enabled.connect('notify::active', lambda widget, _: self.toggle_character(widget, key))
            enabled.set_valign(Gtk.Align.CENTER)
            enabled.get_accessible().set_name('Character enabled')
            control = icon('crown') if primary else self.action('primary', 'crown', 'Make primary', lambda *_: self.set_primary(None, key))
            control.set_tooltip_text(_('Primary character') if primary else _('Make primary'))
            self.heading_actions.pack_start(control, False, False, 0)
            self.heading_actions.pack_start(enabled, False, False, 8)
        if record:
            selected_row = box(spacing=12)
            selected_row.set_margin_top(18)
            selected_row.pack_start(label(record['name'], 'section-title', expand=True), True, True, 0)
            selected_row.set_tooltip_text(_('Interaction') if record['kind'] == 'interaction' else _('Loop') if record['mode'] == 'loop' else _('Play once'))
            left.pack_start(selected_row, False, False, 0)
            if self.area != 'library':
                setting = settings(self.theme, key, record)
                trigger = self.register('action_trigger', combo({k: TRIGGERS[k] for k in record['choices']}, setting['trigger']))
                trigger.set_sensitive(setting['enabled'] and not record['required'])
                trigger.connect('changed', lambda widget: self.action_changed(record, trigger=widget.get_active_id()))
                left.pack_start(field('Trigger', trigger), False, False, 0)
                if setting['trigger'] == 'idle':
                    hint = 'Auto idle is off. Set its delay in Theme settings.' if not self.theme['idle_after'] else f"After {self.theme['idle_after']}–{round(self.theme['idle_after']*1.5)} seconds idle"
                    help_text = label(hint, 'muted')
                    help_text.set_tooltip_text(_(hint))
                    left.pack_start(help_text, False, False, 10)
                if record['id'] == 'shake':
                    left.pack_start(label('Requires Drag & recover', 'muted'), False, False, 10)
        if self.area == 'library':
            left.pack_start(self.usage(lambda theme: key in theme['characters'], 'characters'), False, False, 20)
        self.detail_box.pack_start(fields, True, True, 0)
        self.content.pack_start(self.detail_box, False, False, 0)
        if records:
            title = label('Actions · '+str(len(records)), 'section-title')
            title.set_margin_bottom(12)
            fields.pack_start(title, False, False, 0)
            for action in records:
                action_row = css(box(spacing=8), 'action-row')
                if action['id'] == record['id']:
                    css(action_row, 'selected-action')
                button = css(Gtk.Button(), 'action-select')
                row = box(spacing=12)
                row.pack_start(icon('circle-play', 18), False, False, 0)
                row.pack_start(label(action['name'], expand=True), True, True, 0)
                setting = settings(self.theme, key, action) if self.area != 'library' else {'enabled': True, 'trigger': action['default']}
                status = self.register('action_status:'+action['id'], label(TRIGGERS[setting['trigger']] if setting['enabled'] else 'Off', 'muted'))
                row.pack_end(status, False, False, 0)
                button.add(row)
                button.get_accessible().set_name('Preview '+action['name'])
                button.connect('clicked', lambda _, action=action: self.select_action(action['id']))
                self.register('action:'+action['id'], button)
                action_row.pack_start(button, True, True, 0)
                if self.area != 'library':
                    enabled = self.register('action_enabled:'+action['id'], Gtk.Switch(active=setting['enabled']))
                    enabled.set_valign(Gtk.Align.CENTER)
                    enabled.set_sensitive(not action['required'])
                    enabled.set_tooltip_text(_('Required resting pose') if action['required'] else _('Enable %s in this theme') % action['name'])
                    enabled.get_accessible().set_name('Enable '+action['name'])
                    enabled.connect('notify::active', lambda widget, _, action=action: self.action_changed(action, enabled=widget.get_active()))
                    action_row.pack_end(enabled, False, False, 0)
                    if action['id'] == record['id']:
                        self.register('action_enabled', enabled)
                fields.pack_start(action_row, False, False, 0)


    def refresh_action_controls(self):
        from character_actions import actions, settings, TRIGGERS
        self.guard = True
        try:
            for action in actions(self.state.catalog[self.detail]):
                setting = settings(self.theme, self.detail, action)
                self.page.controls['action_enabled:'+action['id']].set_active(setting['enabled'])
                self.page.controls['action_status:'+action['id']].set_text(TRIGGERS[setting['trigger']] if setting['enabled'] else 'Off')
                if action['id'] == self.selected_actions[self.detail]:
                    self.page.controls['action_trigger'].set_sensitive(setting['enabled'] and not action['required'])
        finally:
            self.guard = False
        self.update_actions()


    def bubble_detail(self):
        key = self.detail
        self.heading(self.bubble_name(key), True)
        self.content.pack_start(self.bubble_art(key, True), False, False, 0)
        # This illustrative card follows the handoff; Preview opens the actual material.
        self.heading_actions.pack_start(self.action('preview', 'bubble', 'Preview bubble material', lambda *_: self.preview_bubble(key)), False, False, 0)
        if self.area == 'library':
            self.content.pack_start(self.usage(lambda theme: theme['bubble'] == key, 'assets'), False, False, 20)
        else:
            selected = self.theme['bubble'] == key
            button = icon('circle-check') if selected else self.action('use_bubble', 'check', 'Use this bubble', lambda *_: self.select_bubble(key))
            self.content.pack_start(field('Theme bubble', button, 'bubble'), False, False, 0)
