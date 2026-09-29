#!/usr/bin/env python3
"""Mochi's native manager: navigation and actions over separate Theme services."""
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parent))
import json
import os
import sys
import time

# Forward old desktop entries and already-running Character menus before loading GTK3.
if __name__ == '__main__' and os.environ.get('MOCHI_MANAGER_LEGACY') != '1':
    from manager_launcher import manager_command
    command = manager_command()
    os.execv(command[0], [*command, *sys.argv[1:]])

import gi
gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
from gi.repository import Gdk, Gio, GLib, Gtk
from manager_commands import CommandRunner
from manager_controller import ManagerController
from config_sync import ConfigSyncService
from manager_bootstrap import ControllerBootstrap
from config_sync_gio import GioConfigEventSource, GlibScheduler
from manager_i18n import _, error_parts
from manager_state import AppError, ImportResult, JobKind, ThemeDraft
from manager_runtime_projection import project_runtime
from manager_view import ManagerView, PageControls
from manager_widgets import icon
from theme_store import ROOT


class Manager(ManagerView):
    @property
    def theme(self):
        return self.state.draft.value

    @theme.setter
    def theme(self, value):
        self.state.draft = ThemeDraft(value, store_revision=self.controller.store.revision)

    @property
    def live(self):
        return self.state.runtime.snapshot

    @live.setter
    def live(self, value):
        self.state.runtime.update(value)

    @property
    def closed(self):
        return self.state.closed

    @closed.setter
    def closed(self, value):
        self.state.closed = value

    @property
    def selected_actions(self):
        return self.state.route.selected_actions

    @selected_actions.setter
    def selected_actions(self, value):
        self.state.route.selected_actions = value

    @property
    def model_catalog(self):
        return self.state.model_catalog

    @model_catalog.setter
    def model_catalog(self, value):
        self.state.model_catalog = value

    @property
    def area(self):
        return self.state.route.area

    @area.setter
    def area(self, value):
        self.state.route.area = value

    @property
    def screen(self):
        return self.state.route.screen

    @screen.setter
    def screen(self, value):
        self.state.route.screen = value

    @property
    def tab(self):
        return self.state.route.tab

    @tab.setter
    def tab(self, value):
        self.state.route.tab = value

    @property
    def library_tab(self):
        return self.state.route.library_tab

    @library_tab.setter
    def library_tab(self, value):
        self.state.route.library_tab = value

    @property
    def detail(self):
        return self.state.route.detail

    @detail.setter
    def detail(self, value):
        self.state.route.detail = value

    @property
    def pending(self):
        return bool(self.runner.active or self.runner.queue)

    @property
    def queued(self):
        return self.runner.queue[0] if self.runner.queue else None

    @property
    def loading(self):
        return self.state.commands.visible_message is not None

    @loading.setter
    def loading(self, value):
        if not value:
            self.state.commands.visible_message = None
        elif not self.state.commands.visible_message:
            self.state.commands.visible_message = 'Working…'

    def __init__(self, app=None, store=None, runtime=None, controller=None):
        self.controller = controller or ManagerController(store, runtime)
        self.state = self.controller.state
        self.store = self.controller.store  # Compatibility for external callers.
        self.runtime = self.controller.runtime
        self.preferences = self.controller.preferences
        self.closed = self.guard = False
        self.area, self.screen, self.tab, self.library_tab = 'themes', 'theme', 'characters', 'characters'
        self.detail = None
        self.width_band = 2
        self.grids = []
        self.detail_box = self.category_bar = None
        self.catalog_requested = False
        self.catalog_job = None
        self.save_timer = None
        self.close_dialog = None
        self.conflict_dialog = None
        self.plugin_review_dialog = None
        self.material_preview = None
        self.action_preview = None
        self.selected_actions = {}
        self.runner = CommandRunner(self.state.commands, lambda callback, *args: GLib.idle_add(callback, *args))
        self.shell = PageControls('shell')
        self.page = None
        self.window = Gtk.ApplicationWindow(application=app, title='Mochi Manager (Legacy)') if app else Gtk.Window(title='Mochi Manager (Legacy)')
        self.window.set_wmclass('mochi-manager', 'MochiManager')
        self.window.set_icon_from_file(str(ROOT/'assets/brand/mochi-icon.svg'))
        self.window.set_default_size(960, 660)
        self.window.set_size_request(420, 560)
        self.window.set_position(Gtk.WindowPosition.CENTER)
        self.window.get_style_context().add_class('mochi-manager')
        self.window.connect('delete-event', self.close)
        self.window.connect('key-press-event', self.key)
        self.window.connect('unmap', lambda *_: self.stop_action_preview())
        self.provider = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_screen(self.window.get_screen(), self.provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.system_dark = self.window.get_settings().get_property('gtk-application-prefer-dark-theme')
        self.desktop_settings = None
        schema = Gio.SettingsSchemaSource.get_default().lookup('org.gnome.desktop.interface', True)
        if schema and schema.has_key('color-scheme'):
            self.desktop_settings = Gio.Settings.new('org.gnome.desktop.interface')
            self.color_handler = self.desktop_settings.connect('changed::color-scheme', lambda *_: self.appearance())
        self.appearance()
        self.build()
        self.render()
        self.config_sync = ConfigSyncService(self.controller.store.file, self.controller.preferences.path,
                                             GioConfigEventSource(), GlibScheduler(), self.config_changed,
                                             resources_path=self.controller.store.resources.path)
        self.timer = GLib.timeout_add(1200, self.poll)
        self.refresh()

    def config_changed(self, change):
        if self.closed:
            return
        try:
            if change.kind == 'resources':
                def loaded(result, error):
                    if self.closed:
                        return
                    if error:
                        self.show_error(error)
                    else:
                        self.controller.adopt_resources(result)
                        self.config_sync.observe_packages(result.plugin_directories)
                        self.render()
                self.runner.submit(JobKind.CATALOG, self.controller.load_resources, loaded)
                return
            if change.kind == 'themes':
                result = self.controller.sync_theme()
                if result == 'pending':
                    if self.save_timer is not None:
                        GLib.source_remove(self.save_timer)
                        self.save_timer = None
                    self.show_theme_update_banner()
                elif result == 'reloaded':
                    self.hide_config_banner()
                    self.render()
            else:
                self.controller.sync_preferences()
                self.appearance()
                if self.controller.translation_conflict:
                    self.show_translation_conflict_banner()
                elif self.area == 'settings':
                    self.render()
            self.update_actions()
        except (OSError, ValueError) as error:
            self.show_error(error)

    def show_config_banner(self, message, actions):
        self.config_banner_label.set_text(_(message))
        for child in self.config_banner_actions.get_children():
            child.destroy()
        for title, callback in actions:
            button = Gtk.Button(label=_(title))
            button.connect('clicked', lambda _button, action=callback: action())
            self.config_banner_actions.pack_start(button, False, False, 0)
        self.config_banner.show_all()

    def hide_config_banner(self):
        self.config_banner.hide()

    def show_theme_update_banner(self):
        self.show_config_banner('Theme updated in another window. Your draft is kept.', [
            ('Reload latest', self.reload_external_theme),
            ('Save as new Theme', self.save_external_as_new),
            ('Keep editing', self.hide_config_banner)])

    def reload_external_theme(self):
        self.controller.reload_latest()
        self.hide_config_banner()
        self.render()

    def save_external_as_new(self):
        try:
            self.controller.save_as_new()
        except (OSError, ValueError) as error:
            self.show_error(error)
            return
        self.hide_config_banner()
        self.render()

    def show_translation_conflict_banner(self):
        self.show_config_banner('Translation settings changed in another window.', [
            ('Use latest', lambda: self.resolve_translation('latest')),
            ('Keep mine', lambda: self.resolve_translation('mine')),
            ('Compare', lambda: self.resolve_translation('compare'))])

    def resolve_translation(self, action):
        try:
            conflict = self.controller.resolve_translation_conflict(action)
            if action == 'compare':
                if conflict:
                    self.show_translation_compare(conflict)
            else:
                self.hide_config_banner()
                self.render()
        except (OSError, ValueError) as error:
            self.show_error(error)

    def show_translation_compare(self, conflict):
        dialog = Gtk.MessageDialog(transient_for=self.window, modal=False,
                                   message_type=Gtk.MessageType.INFO,
                                   buttons=Gtk.ButtonsType.CLOSE,
                                   text=_('Compare translation settings'))
        dialog.format_secondary_text(_('Mine') + ': ' + str(conflict['mine'])[:240] + '\n' +
                                     _('Latest') + ': ' + str(conflict['latest'])[:240])
        dialog.connect('response', lambda widget, *_: widget.destroy())
        dialog.show_all()

    def register(self, name, widget):
        return (self.page or self.shell).register(name, widget)

    def select_action(self, action):
        self.selected_actions[self.detail] = action
        self.render()
        self.preview_action()

    def stop_action_preview(self):
        if self.action_preview:
            self.action_preview.stop()
            self.action_preview = None
        self.runner.mark_external(JobKind.PREVIEW, False)
        for name, sensitive in [('action_play', True), ('action_stop', False)]:
            if self.page and name in self.page.controls:
                self.page.controls[name].set_sensitive(sensitive)

    def preview_action(self):
        from action_preview import ActionPreview
        from character_actions import actions
        self.stop_action_preview()
        try:
            record = next(a for a in actions(self.state.catalog[self.detail]) if a['id'] == self.selected_actions[self.detail])
            def failed(error):
                self.stop_action_preview()
                self.show_preview_error(error)
            self.action_preview = ActionPreview(self.controller.preview_binary(), self.state.catalog[self.detail]['path'],
                                                record, self.action_picture, failed)
            self.runner.mark_external(JobKind.PREVIEW, True)
            self.page.controls['action_play'].set_sensitive(False)
            self.page.controls['action_stop'].set_sensitive(True)
            self.error.hide()
        except (OSError, ValueError) as error:
            self.show_preview_error(error)

    def show_preview_error(self, error):
        issue = self.controller.preview_error(error)
        self.state.commands.last_error_code = issue.code
        self.show_error(issue)

    def action_changed(self, action, **changes):
        if self.guard or self.area == 'library':
            return
        try:
            self.controller.edit_action(self.detail, action, **changes)
            self.error.hide()
        except (OSError, ValueError) as error:
            self.show_error(error)
        if set(changes) == {'enabled'}:
            self.refresh_action_controls()
        else:
            self.render()

    def appearance(self):
        selected = self.state.settings['appearance']
        scheme = self.desktop_settings.get_string('color-scheme') if self.desktop_settings else 'default'
        system = (scheme == 'prefer-dark' if scheme != 'default' else
                  self.system_dark or 'dark' in self.window.get_settings().get_property('gtk-theme-name').lower())
        dark = selected == 'dark' or selected == 'system' and system
        self.window.get_settings().set_property('gtk-application-prefer-dark-theme', dark)
        colors = json.loads((ROOT/'assets/manager/design-tokens.json').read_text())['colors']
        definitions = '\n'.join('@define-color mc_'+key.replace('-', '_')+' '+value['dark' if dark else 'light']+';' for key, value in colors.items())
        self.provider.load_from_data((definitions+'\n'+(ROOT/'assets/manager/style.css').read_text()).encode())

    def app_changed(self, **changes):
        if self.guard:
            return
        try:
            self.controller.update_settings(**changes)
            self.appearance()
            self.error.hide()
        except (OSError, ValueError) as error:
            self.show_error(error)

    def navigate(self, area):
        if self.loading:
            return
        self.flush_save()
        self.area, self.screen, self.detail = area, 'list' if area == 'themes' else 'theme', None
        self.error.hide()
        self.render()

    def open_theme(self, key, tab=None):
        if self.loading:
            return
        try:
            self.flush_save()
            if self.state.draft.needs_save:
                return
            self.controller.choose(key)
            self.area, self.screen, self.detail = 'themes', 'theme', None
            if tab:
                self.tab = tab
            self.error.hide()
            self.render()
        except (OSError, ValueError) as error:
            self.show_error(error)

    def show_screen(self, screen):
        self.flush_save()
        self.screen, self.detail = screen, None
        self.error.hide()
        self.render()

    def open_detail(self, key):
        self.flush_save()
        self.detail = key
        self.error.hide()
        self.render()
        if self.area == 'themes' and self.tab == 'plugins' and key == 'translation' and not self.catalog_requested:
            self.refresh_models()

    def back(self, *_):
        if self.screen == 'new':
            self.navigate('themes')
        else:
            self.detail, self.screen = None, 'theme'
            self.error.hide()
            self.render()

    def change_tab(self, tab):
        self.flush_save()
        if self.area == 'library':
            self.library_tab = tab
        else:
            self.tab = tab
        self.detail, self.screen = None, 'theme'
        self.error.hide()
        self.render()
        self.page.controls['tab:'+tab].grab_focus()

    def tab_key(self, widget, event):
        if event.keyval not in (Gdk.KEY_Left, Gdk.KEY_Right, Gdk.KEY_Home, Gdk.KEY_End):
            return False
        index = self.tab_buttons.index(widget)
        index = 0 if event.keyval == Gdk.KEY_Home else len(self.tab_buttons)-1 if event.keyval == Gdk.KEY_End else (index+(1 if event.keyval == Gdk.KEY_Right else -1)) % len(self.tab_buttons)
        self.tab_buttons[index].clicked()
        return True

    def create(self, *_):
        try:
            self.flush_save()
            if self.state.draft.needs_save:
                return
            name = self.page.controls['new_name'].get_text().strip()
            if not name:
                raise ValueError('Enter a theme name.')
            self.controller.create(name)
            self.tab = 'characters'
            self.screen = 'theme'
            self.render()
        except (OSError, ValueError) as error:
            self.show_error(error)

    def save(self, rebuild=True):
        if self.save_timer is not None:
            GLib.source_remove(self.save_timer)
            self.save_timer = None
        try:
            self.controller.save()
            self.error.hide()
        except (OSError, ValueError) as error:
            self.show_error(error)
        if rebuild:
            self.render()
        else:
            self.update_actions()

    def edit_theme(self, **changes):
        if not self.guard:
            self.controller.edit_theme(**changes)
            if set(changes) <= {'name', 'size'}:
                self.schedule_save()
                self.update_actions()
            else:
                self.save(rebuild=False)

    def schedule_save(self):
        if self.save_timer is not None:
            GLib.source_remove(self.save_timer)
        def delayed():
            self.save_timer = None
            self.flush_save()
            return False
        self.save_timer = GLib.timeout_add(350, delayed)

    def flush_save(self):
        if self.save_timer is not None:
            GLib.source_remove(self.save_timer)
            self.save_timer = None
        if not self.state.draft.needs_save:
            return True
        try:
            self.controller.save()
            self.error.hide()
        except (OSError, ValueError) as error:
            self.show_error(error)
            self.update_actions()
            return False
        self.update_actions()
        return True

    def name_changed(self, widget):
        if self.guard:
            return
        if self.theme['name'] != widget.get_text():
            self.controller.edit_theme(name=widget.get_text())
        self.flush_save()
        widget.set_text(self.theme['name'])
        self.breadcrumbs()

    def name_typed(self, widget):
        if self.guard or self.theme['name'] == widget.get_text():
            return
        self.controller.edit_theme(name=widget.get_text())
        if widget.get_text().strip():
            self.schedule_save()
        self.breadcrumbs()
        self.update_actions()

    def add_characters(self, *_):
        if len(self.theme['characters']) >= 4:
            self.show_error('This theme supports four characters. Disable one before adding another.')
        else:
            self.tab = 'characters'
            self.show_screen('add')

    def add_character(self, key):
        try:
            self.controller.add_character(key)
            self.error.hide()
            self.detail, self.screen = key, 'theme'
            self.render()
        except (OSError, ValueError) as error:
            self.show_error(error)

    def toggle_character(self, button, key):
        if self.guard:
            return
        if button.get_active():
            self.add_character(key)
        else:
            try:
                self.controller.remove_character(key)
                self.error.hide()
                self.render()
            except (OSError, ValueError) as error:
                self.show_error(error)
                self.render()

    def set_primary(self, _button, key):
        try:
            self.controller.set_primary(key)
            self.error.hide()
            self.render()
        except (OSError, ValueError) as error:
            self.show_error(error)

    def plugin_changed(self, widget, key):
        if not self.guard:
            review = self.controller.plugin_review(key) if widget.get_active() else None
            if review:
                self.guard = True
                widget.set_active(key in self.theme['plugins'])
                self.guard = False
                self.confirm_plugin(review)
                return
            try:
                self.controller.set_plugin_enabled(key, widget.get_active())
                self.error.hide()
                self.update_actions()
                widget.set_active(key in self.theme['plugins'])
            except (OSError, ValueError) as error:
                self.show_error(error)
                self.guard = True
                widget.set_active(key in self.theme['plugins'])
                self.guard = False
                self.update_actions()

    def confirm_plugin(self, review):
        if self.plugin_review_dialog:
            self.plugin_review_dialog.present()
            return
        from manager_plugin_review import review_details
        theme_id = self.theme['id']
        dialog = Gtk.MessageDialog(transient_for=self.window, modal=True,
                                   buttons=Gtk.ButtonsType.NONE, text=_('Approve and enable plugin?'))
        self.plugin_review_dialog = dialog
        dialog.set_destroy_with_parent(True)
        dialog.format_secondary_text(review_details(review))
        dialog.add_button(_('Cancel'), Gtk.ResponseType.CANCEL)
        dialog.add_button(_('Approve and enable'), Gtk.ResponseType.ACCEPT)
        def answered(widget, response):
            widget.destroy()
            self.plugin_review_dialog = None
            if response != Gtk.ResponseType.ACCEPT or self.closed:
                return
            def approved(result, error):
                if self.closed:
                    return
                if error:
                    self.show_error(error)
                else:
                    self.state.plugins[review['id']] = result
                    if self.theme['id'] == theme_id:
                        try:
                            self.controller.set_plugin_enabled(review['id'], True)
                        except (OSError, ValueError) as failure:
                            self.show_error(failure)
                    self.render()
            self.runner.submit(JobKind.CATALOG,
                lambda: self.controller.approve_plugin(review['id'], review['package_sha256']), approved)
        dialog.connect('response', answered)
        dialog.show_all()

    def plugin_setting_changed(self, key, identity, value):
        if not self.guard and value is not None:
            self.controller.edit_plugin_setting(key, identity, value)
            self.schedule_save()
            self.update_actions()

    def select_bubble(self, key):
        try:
            self.controller.set_bubble(key)
            self.error.hide()
            self.render()
        except (OSError, ValueError) as error:
            self.show_error(error)

    def translation_changed(self, **changes):
        if self.guard or any(value is None for value in changes.values()):
            return
        try:
            if 'model' in changes and changes['model'] in self.model_catalog and not self.model_catalog[changes['model']].get('fast_tier'):
                changes['fast'] = False
            self.controller.edit_translation(self.theme['id'], **changes)
            self.sync_fast()
            self.error.hide()
            self.update_actions()
        except (OSError, ValueError) as error:
            self.show_error(error)
            if isinstance(error, AppError) and error.code == 'translation.conflict':
                self.show_translation_conflict_banner()

    def refresh_models(self, *_):
        if self.catalog_job:
            return
        self.catalog_requested = True
        def failed(error):
            error = self.controller.catalog_error(error)
            self.controller.stop_model_catalog(self.catalog_job)
            self.catalog_job = None
            self.runner.mark_external(JobKind.CATALOG, False)
            self.state.commands.last_error_code = error.code
            if not self.closed:
                if 'models_refresh' in self.page.controls:
                    self.page.controls['models_refresh'].set_sensitive(True)
                self.show_error(error)
        def received(event):
            try:
                self.model_catalog = self.controller.parse_model_catalog(event)
            except ValueError as error:
                failed(error)
                return
            self.controller.release_model_catalog(self.catalog_job)
            self.catalog_job = None
            self.runner.mark_external(JobKind.CATALOG, False)
            if not self.closed and 'model' in self.page.controls:
                # Catalog reads must not rebuild the page and discard typing.
                model = self.page.controls['model']
                entry = model.get_child()
                selected, text, position = model.get_active_id(), entry.get_text(), entry.get_position()
                self.guard = True
                model.remove_all()
                for key in self.model_catalog:
                    model.append(key, key)
                if selected:
                    if selected not in self.model_catalog:
                        model.append(selected, selected)
                    model.set_active_id(selected)
                else:
                    entry.set_text(text)
                    entry.set_position(position)
                self.guard = False
                self.sync_fast()
                self.page.controls['models_refresh'].set_sensitive(True)
        try:
            self.catalog_job = self.controller.start_model_catalog(received, failed)
            self.runner.mark_external(JobKind.CATALOG, True)
            self.page.controls['models_refresh'].set_sensitive(False)
        except OSError as error:
            failed(error)

    def sync_fast(self):
        if 'fast' not in self.page.controls:
            return
        options = self.controller.translation(self.theme['id'])
        fast = self.page.controls['fast']
        guard, self.guard = self.guard, True
        fast.set_active(options['fast'])
        self.guard = guard
        supported = bool(self.model_catalog.get(options['model'], {}).get('fast_tier'))
        fast.set_sensitive(not self.model_catalog or supported or options['fast'])
        fast.set_tooltip_text(_('Faster responses may increase usage.') if not self.model_catalog or supported else _('This model does not offer Fast. Choose another model or turn Fast off.'))

    def apply(self, *_):
        if self.state.runtime.disconnected and self.state.runtime.snapshot is None:
            return
        if not self.flush_save():
            return
        theme = self.state.draft.snapshot()
        options = self.controller.translation(theme['id'])
        def applied():
            if self.live and self.live.get('translation') == options:
                self.controller.mark_translation_applied(theme['id'], options)
        self.work(lambda: self.controller.apply(theme, options), 'Applying…', done=applied)

    def stop_theme(self):
        self.work(self.controller.stop, 'Stopping…')

    def update_actions(self):
        options = self.controller.translation(self.theme['id'])
        presentation = project_runtime(self.state.runtime, self.theme, options, self.controller.same_config)
        current, same, applied, dirty = (presentation.running_theme, presentation.same_theme,
                                         presentation.applied, presentation.dirty)
        hint = _(presentation.action)
        self.start.set_tooltip_text(hint)
        self.start.get_accessible().set_name(hint)
        symbol = 'check' if dirty and not presentation.degraded else 'play'
        if getattr(self, 'start_symbol', None) != symbol:
            self.start_icon.set_symbol(symbol)
            self.start_symbol = symbol
        self.start_label.set_visible(bool(dirty))
        self.start.set_visible(not applied and bool(presentation.action))
        self.start.set_sensitive(not self.loading and bool(presentation.action))
        self.stop.set_visible(bool(same))
        self.stop.set_sensitive(bool(current) and not self.loading)
        self.pending_label.set_visible(bool(dirty))
        self.content.set_sensitive(not self.loading)
        self.sidebar.set_sensitive(not self.loading)
        self.crumbs.set_sensitive(not self.loading)
        self.footer.set_visible(self.area == 'themes' and self.screen not in ('list', 'new'))
        if not self.loading:
            self.spinner.stop()
            self.spinner.hide()
            self.status.set_text(_('%s is running') % self.theme_name(current) if presentation.status == 'Another Theme is running'
                                 else _(presentation.status))
            self.status.set_tooltip_text(self.status.get_text())
        state = 'circle-play' if same else 'layers' if current else 'circle-stop'
        if getattr(self, 'state_symbol', None) != state:
            self.state_icon.set_symbol(state)
            self.state_symbol = state
        context = self.state_icon.get_style_context()
        (context.add_class if current else context.remove_class)('live')

    def preview_bubble(self, key):
        from messages import Content, Message, MessageState
        from speech_bubble import SpeechBubble
        if self.material_preview:
            self.material_preview.destroy()
            self.material_preview = None
        def dismiss(*_):
            if self.material_preview:
                self.material_preview.destroy()
                self.material_preview = None
            self.runner.mark_external(JobKind.PREVIEW, False)
        try:
            preview = SpeechBubble(self.controller.bubble_material(key), dismiss, lambda *_: None, lambda: None)
            self.material_preview = preview
            self.runner.mark_external(JobKind.PREVIEW, True)
            preview.window.set_transient_for(self.window)
            preview.window.set_position(Gtk.WindowPosition.CENTER_ON_PARENT)
            preview.show()
            preview.render(Message('preview', 'manager', 'manager', 'preview', Content('Mochi', 'A little company for your day.', kind='notice'), MessageState.INFO, 0, time.monotonic(), None))
        except (OSError, ValueError, GLib.Error) as error:
            dismiss()
            self.show_preview_error(error)

    def import_file(self, kind):
        dialog = Gtk.FileChooserNative(title=_({'theme': 'Import theme configuration', 'character': 'Import character package', 'bubble': 'Import bubble material', 'plugin': 'Import plugin package'}[kind]),
                                       transient_for=self.window, action=Gtk.FileChooserAction.OPEN, accept_label=_('Import'), cancel_label=_('Cancel'))
        file_filter = Gtk.FileFilter()
        file_filter.set_name(_('Theme configuration') if kind == 'theme' else _('ZIP plugin package') if kind == 'plugin' else _('ZIP material package'))
        file_filter.add_pattern('*.json' if kind == 'theme' else '*.zip')
        dialog.add_filter(file_filter)
        def response(chooser, result):
            path = chooser.get_filename()
            chooser.destroy()
            if result != Gtk.ResponseType.ACCEPT or not path:
                return
            self.flush_save()
            def loaded(result):
                self.controller.after_import()
                if kind == 'theme':
                    self.area, self.screen, self.detail = 'themes', 'theme', None
                self.render()
                if result.partial:
                    self.show_error(result.issue)
            self.work(lambda: self.controller.import_file(kind, path), 'Importing…', done=loaded, kind=JobKind.IMPORT)
        dialog.connect('response', response)
        dialog.show()

    def export_theme(self):
        self.flush_save()
        dialog = Gtk.FileChooserNative(title=_('Export theme configuration'), transient_for=self.window,
                                       action=Gtk.FileChooserAction.SAVE, accept_label=_('Export'), cancel_label=_('Cancel'))
        dialog.set_current_name(self.theme['id']+'.theme.json')
        dialog.set_do_overwrite_confirmation(True)
        def response(chooser, result):
            path = chooser.get_filename()
            chooser.destroy()
            if result == Gtk.ResponseType.ACCEPT and path:
                try:
                    self.controller.export_theme(path)
                except OSError as error:
                    self.show_error(error)
        dialog.connect('response', response)
        dialog.show()

    def show_error(self, error):
        summary, detail = error_parts(error)
        brief = detail.replace('\n', ' ').strip()
        self.error.set_text((summary + (' — ' + brief[:180] + ('…' if len(brief) > 180 else '') if brief else ''))[:350])
        self.error_detail_text.get_buffer().set_text(detail[:8192] + ('\n…' if len(detail) > 8192 else ''))
        self.error_details.set_expanded(False)
        self.error_details.set_visible(bool(detail))
        self.error.show()
        if isinstance(error, AppError) and error.code == 'theme.conflict':
            self.show_conflict()

    def show_conflict(self):
        if self.conflict_dialog:
            self.conflict_dialog.present()
            return
        dialog = Gtk.MessageDialog(transient_for=self.window, modal=True,
                                   message_type=Gtk.MessageType.WARNING,
                                   buttons=Gtk.ButtonsType.NONE,
                                   text=_('Theme changed in another window'))
        dialog.format_secondary_text(_('Your edits are still here. Choose how to resolve the newer saved version.'))
        dialog.add_button(_('Keep editing'), Gtk.ResponseType.CANCEL)
        dialog.add_button(_('Reload latest'), Gtk.ResponseType.NO)
        dialog.add_button(_('Save as new Theme'), Gtk.ResponseType.ACCEPT)
        self.conflict_dialog = dialog
        def response(widget, choice):
            widget.destroy()
            self.conflict_dialog = None
            if choice == Gtk.ResponseType.CANCEL:
                return
            try:
                if choice == Gtk.ResponseType.NO:
                    self.controller.reload_latest()
                elif choice == Gtk.ResponseType.ACCEPT:
                    self.controller.save_as_new()
                self.error.hide()
                self.render()
            except (OSError, ValueError) as failure:
                self.show_error(failure)
        dialog.connect('response', response)
        dialog.show_all()

    def work(self, function, message=None, refresh=False, done=None, kind=None):
        if self.closed:
            return
        job_kind = kind or (JobKind.STATUS if message is None else JobKind.RUNTIME)
        started_job = False

        def started():
            nonlocal started_job
            started_job = True
            if message is None:
                return
            self.state.commands.visible_message = message
            self.error.hide()
            self.update_actions()
            self.spinner.show()
            self.spinner.start()
            self.status.set_text(message)

        def perform():
            result = function()
            return self.controller.status() if refresh else result

        def completed(result, error):
            self.finish(result, error, done, message is not None and started_job, job_kind)

        self.runner.submit(job_kind, perform, completed, started)

    def finish(self, result, error, done, was_loading=False, kind=JobKind.RUNTIME):
        if was_loading:
            self.state.commands.visible_message = None
        if self.closed:
            return False
        cancelled = error is not None and str(error) == 'Cancelled.'
        if error and not cancelled:
            self.show_error(error)
            if kind in (JobKind.RUNTIME, JobKind.STATUS):
                self.state.runtime.failed(error.code)
        elif kind == JobKind.IMPORT and isinstance(result, ImportResult):
            self.state.last_import = result
        elif not cancelled:
            self.live = result
        if done and not error:
            try:
                done(result) if kind == JobKind.IMPORT else done()
            except (OSError, ValueError) as failure:
                self.show_error(failure)
        if self.closed:
            return False
        if was_loading:
            self.render()
        else:
            self.update_actions()
        if kind == JobKind.IMPORT and not error and isinstance(result, ImportResult):
            self.refresh()
        if self.state.commands.closing and not self.pending:
            if error and str(error) != 'Cancelled.':
                self.state.commands.closing = False
            else:
                self.finish_close()
        return False

    def refresh(self):
        self.work(self.controller.status, kind=JobKind.STATUS)

    def poll(self):
        if self.closed:
            return False
        if self.window.get_visible() and not self.pending:
            self.refresh()
        return True

    def key(self, _window, event):
        if event.keyval == Gdk.KEY_Escape and (self.detail or self.screen in ('options', 'add', 'replace', 'new')):
            self.back()
            return True
        return False

    def close(self, *_):
        if self.closed:
            return True
        self.flush_save()
        if self.state.draft.needs_save:
            return True
        if self.pending or self.catalog_job:
            self.show_close_status()
            return True
        self.finish_close()
        return True

    def show_close_status(self):
        if self.close_dialog:
            self.close_dialog.present()
            return
        kind = self.state.commands.active
        catalog_only = not self.pending and self.catalog_job is not None
        name = _('Loading models…') if catalog_only else self.state.commands.visible_message or {
            JobKind.STATUS: 'Refreshing status…',
            JobKind.IMPORT: 'Importing…',
            JobKind.RUNTIME: 'Applying changes…',
        }.get(kind, 'Working…')
        dialog = Gtk.MessageDialog(transient_for=self.window, modal=True,
                                   message_type=Gtk.MessageType.INFO,
                                   buttons=Gtk.ButtonsType.NONE,
                                   text=_('Mochi is still working'))
        dialog.format_secondary_text((_('%s You can keep the window open, or cancel the model lookup and close safely.') if catalog_only else
                                      _('%s You can keep the window open, or close it when the current task finishes. Queued tasks can be cancelled safely.')) % name)
        dialog.add_button(_('Keep open'), Gtk.ResponseType.CANCEL)
        dialog.add_button(_('Cancel lookup and close') if catalog_only else _('Close when finished'), Gtk.ResponseType.ACCEPT)
        if self.runner.queue:
            dialog.add_button(_('Cancel queued tasks and close'), Gtk.ResponseType.APPLY)
        self.close_dialog = dialog

        def response(widget, choice):
            widget.destroy()
            self.close_dialog = None
            if choice == Gtk.ResponseType.APPLY:
                self.state.commands.closing = True
                self.runner.cancel_pending()
            elif choice == Gtk.ResponseType.ACCEPT:
                if catalog_only:
                    self.finish_close()
                else:
                    self.state.commands.closing = True
            if self.state.commands.closing:
                self.content.set_sensitive(False)
                self.sidebar.set_sensitive(False)
                self.crumbs.set_sensitive(False)
                self.status.set_text(_('Closing when task finishes…'))
            if self.state.commands.closing and not self.pending:
                self.finish_close()

        dialog.connect('response', response)
        dialog.show_all()

    def finish_close(self):
        if self.closed:
            return
        if self.catalog_job:
            self.controller.stop_model_catalog(self.catalog_job)
            self.catalog_job = None
            self.runner.mark_external(JobKind.CATALOG, False)
        self.stop_action_preview()
        if self.state.settings['close'] == 'quit':
            self.state.commands.closing = False
            self.work(self.controller.stop, 'Stopping…', done=self.dispose)
        else:
            self.dispose()

    def dispose(self):
        if self.closed:
            return
        self.flush_save()
        self.closed = True
        self.guard = True
        if self.close_dialog:
            self.close_dialog.destroy()
            self.close_dialog = None
        if self.conflict_dialog:
            self.conflict_dialog.destroy()
            self.conflict_dialog = None
        self.stop_action_preview()
        self.config_sync.close()
        GLib.source_remove(self.timer)
        if self.desktop_settings:
            self.desktop_settings.disconnect(self.color_handler)
        if self.material_preview:
            self.material_preview.destroy()
            self.material_preview = None
        if self.catalog_job:
            self.controller.stop_model_catalog(self.catalog_job)
            self.catalog_job = None
        if self.plugin_review_dialog:
            self.plugin_review_dialog.destroy()
            self.plugin_review_dialog = None
        self.runner.shutdown()
        self.controller.shutdown()
        Gtk.StyleContext.remove_provider_for_screen(self.window.get_screen(), self.provider)
        application = self.window.get_application()
        self.window.destroy()
        if application is None and Gtk.main_level() > 0:
            Gtk.main_quit()


def manager_application_flags():
    return Gio.ApplicationFlags.NON_UNIQUE if os.environ.get('MOCHI_MANAGER_NON_UNIQUE') == '1' else Gio.ApplicationFlags.DEFAULT_FLAGS


def main():
    GLib.set_prgname('io.mochi.Manager')
    GLib.set_application_name('Mochi')
    app = Gtk.Application(application_id='io.mochi.Manager', flags=manager_application_flags())
    managers = []
    startup = ControllerBootstrap(lambda callback, *args: GLib.idle_add(callback, *args))
    opening = False
    def activate(application):
        nonlocal opening
        if managers and not managers[0].closed:
            managers[0].window.present()
            return
        if opening:
            return
        opening = True
        application.hold()
        def ready(controller, error):
            nonlocal opening
            opening = False
            try:
                if error:
                    dialog = Gtk.MessageDialog(message_type=Gtk.MessageType.ERROR, buttons=Gtk.ButtonsType.CLOSE, text=_('Could not open Mochi'))
                    dialog.format_secondary_text(str(error))
                    dialog.run()
                    dialog.destroy()
                    return False
                manager = Manager(application, controller=controller)
                managers[:] = [manager]
                manager.window.show_all()
                manager.error.hide()
                manager.render()
            finally:
                application.release()
            return False
        startup.start(ready)
    app.connect('activate', activate)
    try:
        return app.run(sys.argv)
    finally:
        startup.close()


if __name__ == '__main__':
    raise SystemExit(main())
