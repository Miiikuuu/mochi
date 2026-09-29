#!/usr/bin/env python3
"""Parallel GTK4 entry for the Mochi Manager v2 shell."""
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parent))
import os
import sys
from datetime import datetime, timezone

import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gio, GLib, Gtk

from manager_commands import CommandRunner
from resource_tasks import ResourceTask
from manager_mutation_presentation import project_mutations
from manager_bootstrap import ControllerBootstrap
from config_sync import ConfigSyncService
from config_sync_gio import GioConfigEventSource, GlibScheduler
from manager_i18n import _, configure_language
from manager_theme_edit import ThemeEditCoordinator
from manager_runtime_projection import project_runtime
from manager_state import AppError, CommandState, JobKind, error_from_exception
from manager_v2_style import layout_tokens, stylesheet
from manager_v2_view import ManagerV2View


class ManagerV2:
    def __init__(self, app, controller):
        self.controller = controller
        self.state = self.controller.state
        self.state.route.area = 'themes'
        self.state.route.tab = 'preview'
        self.runner = CommandRunner(self.state.commands, lambda callback, *args: GLib.idle_add(callback, *args))
        self.resource_runner = CommandRunner(CommandState(), lambda callback, *args: GLib.idle_add(callback, *args))
        self.library_generation = 0
        self.library_error = None
        self.library_job = None
        self.resource_task_sequence = 0
        self.resource_tasks = {}
        self.library_file_dialog = None
        self.resources_ready = False
        self.resource_reload_pending = False
        self.plugin_review_dialog = None
        self.close_dialog = None
        self.settings_dialog = None
        self.diagnostics_dialog = None
        self.catalog_job = None
        self.catalog_requested = False
        self.conflict_dialog = None
        self.disposed = False
        self.generation = 0
        self.system_dark = bool(Gtk.Settings.get_default().get_property('gtk-application-prefer-dark-theme'))
        self.state.settings['appearance'] = os.environ.get('MOCHI_MANAGER_V2_APPEARANCE', self.state.settings.get('appearance', 'system'))
        dark = self.apply_appearance()
        handlers = {'navigate': self.navigate, 'choose': self.choose, 'repair': self.repair_resources,
                    'tab': self.select_tab, 'edit_name': self.edit_name, 'apply': self.primary_action,
                    'cast': self.edit_cast, 'behavior': self.edit_behavior,
                    'select_character': self.select_character, 'bubble': self.edit_bubble,
                    'feature': self.edit_feature, 'translation': self.edit_translation,
                    'setting': self.edit_setting, 'settings_model': self.controller.settings_read_model,
                    'settings_reset': self.reset_settings, 'models_refresh': self.refresh_models,
                    'connection_disconnect': self.disconnect_chatgpt, 'diagnostics': self.show_diagnostics,
                    'library_category': self.library_category, 'library_file': self.library_file,
                    'library_lifecycle': lambda action, kind, key: self.run_resource(action, kind, key=key),
                    'library_review': self.library_review, 'library_remove': self.library_remove,
                    'library_drop': lambda kind, path: self.run_resource('import', kind, path=path),
                    'library_cancel': self.cancel_resource_tasks, 'library_runtime': self.library_runtime}
        self.view = ManagerV2View(app, self.state, handlers, stylesheet(dark),
                                  layout_tokens(), self.controller.brand_icon_bytes())
        self.edits = ThemeEditCoordinator(self.controller, GlibScheduler(),
            refresh=self.refresh_edit, error=self.view.show_error, clear_error=self.view.clear_error,
            conflict=self.show_conflict, show_undo=self.view.show_undo,
            hide_undo=self.view.hide_undo, notice=self.view.show_notice)
        self.view.connect('close-request', self.close)
        self.render_page(self.character_models())
        self.update_runtime()
        self.refresh_status()
        self.config_sync = ConfigSyncService(self.controller.store.file, self.controller.preferences.path,
                                             GioConfigEventSource(), GlibScheduler(), self.config_changed,
                                             resources_path=self.controller.store.resources.path)
        self.session_observed = 0
        self.session_subscription = self.controller.start_connection_events(self.connection_event, lambda fn, *args: GLib.idle_add(fn, *args))
        self.status_timer = GLib.timeout_add_seconds(5, self.poll_status)
        self.refresh_resources()

    @property
    def save_timer(self):
        return self.edits.save_timer

    @property
    def undo_timer(self):
        return self.edits.undo_timer

    @property
    def undo_command(self):
        return self.edits.undo_command

    def refresh_edit(self, full):
        if self.disposed:
            return
        self.view.set_theme_list()
        if full:
            self.render_page(self.character_models())
        else:
            self.view.update_theme_identity()
            self.view.update_preview_size(self.state.draft.value['size'])
        self.update_runtime()

    def config_changed(self, change):
        if self.disposed:
            return
        try:
            if change.kind == 'resources':
                self.refresh_resources()
                return
            if change.kind == 'themes':
                result = self.controller.sync_theme()
                if result in ('pending', 'reloaded'):
                    self.invalidate_undo()
                if result == 'pending':
                    self.edits.cancel_save()
                    self.show_theme_update_banner()
                elif result == 'reloaded':
                    self.view.hide_banner()
                    self.view.set_theme_list()
                    self.render_page(self.character_models() if self.state.route.area == 'themes' else None)
            else:
                old_language = self.state.settings.get('language', 'system')
                self.controller.sync_preferences()
                self.apply_appearance()
                if old_language != self.state.settings.get('language', 'system'):
                    self.refresh_resources()
                    self.render_page()
                if self.state.route.area == 'settings':
                    self.render_page()
                if self.controller.translation_conflict:
                    self.show_translation_conflict_banner()
                elif self.state.route.tab == 'features':
                    self.view.update_translation(self.controller.translation(self.state.draft.value['id']))
            if change.kind == 'themes' and self.state.route.area == 'library':
                self.refresh_resources()
            self.update_runtime()
        except (OSError, ValueError) as error:
            self.view.show_error(error_from_exception(error, 'settings' if change.kind == 'preferences' else 'theme'))

    def refresh_resources(self):
        if self.disposed:
            return
        if self.resource_runner.active:
            self.resource_reload_pending = True
            return
        generation = self.generation
        def loaded(result, error):
            if self.disposed or generation != self.generation:
                return
            if error:
                self.view.show_error(error)
            else:
                previous = self.controller.resource_snapshot
                changed = (previous is None or previous.revision != result.revision or
                           self.state.route.area == 'library' and (previous.theme_revision != result.theme_revision or
                               previous.runtime_dependencies != result.runtime_dependencies or previous.runtime_error != result.runtime_error))
                self.controller.adopt_resources(result)
                self.config_sync.observe_packages(result.plugin_directories)
                self.resources_ready = True
                if changed:
                    self.render_page(self.character_models())
                self.update_runtime()
            if self.state.commands.closing:
                if not self.runner.active and not self.resource_runner.active and not self.resource_runner.queue:
                    self.finish_close()
                return
            if self.resource_reload_pending:
                self.resource_reload_pending = False
                self.refresh_resources()
        self.resource_runner.submit(JobKind.CATALOG, self.controller.load_resources, loaded)

    def apply_appearance(self):
        configure_language(self.state.settings.get('language', 'system'))
        Gtk.Settings.get_default().set_property('gtk-enable-animations', not self.state.settings.get('reduced_motion', False))
        choice = self.state.settings.get('appearance', 'system')
        dark = choice == 'dark' or choice == 'system' and self.system_dark
        Gtk.Settings.get_default().set_property('gtk-application-prefer-dark-theme', bool(dark))
        self.applied_dark = bool(dark)
        if hasattr(self, 'view'):
            for area, title in (('themes', 'Themes'), ('library', 'Library'), ('connections', 'Connections'), ('settings', 'Settings')):
                button = self.view.rail_buttons[area]
                button.set_tooltip_text(_(title))
                button.update_property([Gtk.AccessibleProperty.LABEL], [_(title)])
            self.view.compact_themes.set_label(_('Themes'))
            self.view.set_theme_list()
            self.view.provider.load_from_data(stylesheet(self.applied_dark))
        return self.applied_dark

    def edit_setting(self, field, value):
        try:
            self.controller.update_settings(**{field: value})
            self.apply_appearance()
            if field == 'language':
                self.render_page()
                self.refresh_resources()
        except (ValueError, OSError) as error:
            self.view.show_error(error_from_exception(error, 'settings'))
            if hasattr(self.view.page, 'update_settings'):
                self.view.page.update_settings()

    def reset_settings(self):
        if self.settings_dialog:
            self.settings_dialog.present()
            return
        dialog = Gtk.MessageDialog(transient_for=self.view, modal=True, buttons=Gtk.ButtonsType.NONE,
                                   text=_('Reset Manager preferences?'),
                                   secondary_text=_('Resets appearance, login, language and saved Translation drafts. Themes and resources are kept.'))
        dialog.add_button(_('Cancel'), Gtk.ResponseType.CANCEL)
        dialog.add_button(_('Reset'), Gtk.ResponseType.ACCEPT)
        self.settings_dialog = dialog
        def answered(widget, response):
            widget.destroy()
            self.settings_dialog = None
            if response == Gtk.ResponseType.ACCEPT and not self.disposed:
                try:
                    self.controller.reset_settings()
                    self.apply_appearance()
                    self.render_page()
                    self.view.show_notice('Preferences reset')
                except (ValueError, OSError) as error:
                    self.view.show_error(error_from_exception(error, 'settings'))
        dialog.connect('response', answered)
        dialog.present()

    def refresh_models(self):
        if self.disposed or self.catalog_job is not None or self.state.commands.closing:
            return
        self.catalog_requested = True
        self.state.models.loading = True
        self.state.models.error = None
        generation = self.generation
        settled = False
        def finish(error=None):
            nonlocal settled
            if self.disposed or generation != self.generation or settled:
                return
            settled = True
            job, self.catalog_job = self.catalog_job, None
            self.controller.stop_model_catalog(job) if error else self.controller.release_model_catalog(job)
            self.runner.mark_external(JobKind.CATALOG, False)
            self.state.models.loading = False
            self.state.models.error = self.controller.catalog_error(error).code if error else None
            self.update_runtime()
            if error:
                self.view.show_error(self.controller.catalog_error(error))
        def received(event):
            if self.disposed or generation != self.generation or settled:
                return
            try:
                models = self.controller.parse_model_catalog(event)
            except ValueError as error:
                finish(error)
                return
            self.state.models.models = models
            self.state.model_catalog = models
            self.state.models.last_synced = datetime.now(timezone.utc).isoformat(timespec='seconds')
            finish()
        try:
            self.catalog_job = self.controller.start_model_catalog(received, finish)
            self.runner.mark_external(JobKind.CATALOG, True)
        except (OSError, ValueError) as error:
            finish(error)
        self.update_runtime()

    def connection_event(self, frame):
        if self.disposed or frame.get('_observed_at', 0) < self.session_observed:
            return
        self.session_observed = frame.get('_observed_at', 0)
        self.state.session_connection = frame['state']
        if hasattr(self.view.page, 'update_connections'):
            self.view.page.update_connections()

    def disconnect_chatgpt(self):
        generation = self.generation
        def completed(result, error):
            if self.disposed or self.generation != generation:
                return
            if error:
                self.view.show_error(error)
            else:
                self.connection_event(result)
            self.update_runtime()
        self.runner.submit(JobKind.CATALOG, self.controller.disconnect_chatgpt, completed, self.update_runtime)

    def show_diagnostics(self):
        from manager_connections import project_connections
        if self.diagnostics_dialog:
            self.diagnostics_dialog.present()
            return
        value = project_connections(self.state.runtime, self.state.models, self.state.session_connection).diagnostics()
        dialog = Gtk.MessageDialog(transient_for=self.view, modal=True, buttons=Gtk.ButtonsType.NONE,
                                   text=_('Connection diagnostics'), secondary_text=value)
        dialog.add_button(_('Close'), Gtk.ResponseType.CLOSE)
        dialog.add_button(_('Copy'), Gtk.ResponseType.APPLY)
        self.diagnostics_dialog = dialog
        def answered(widget, response):
            if response == Gtk.ResponseType.APPLY:
                self.view.get_clipboard().set(value)
            else:
                widget.destroy()
                self.diagnostics_dialog = None
        dialog.connect('response', answered)
        dialog.present()

    def show_theme_update_banner(self):
        self.view.show_banner('Theme updated in another window. Your draft is kept.', [
            ('Reload latest', self.reload_external_theme),
            ('Save as new Theme', self.save_external_as_new),
            ('Keep editing', self.view.hide_banner)])

    def reload_external_theme(self):
        self.invalidate_undo()
        self.controller.reload_latest()
        self.view.hide_banner()
        self.view.set_theme_list()
        self.render_page(self.character_models() if self.state.route.area == 'themes' else None)
        self.update_runtime()

    def save_external_as_new(self):
        self.invalidate_undo()
        try:
            self.controller.save_as_new()
        except (OSError, ValueError) as error:
            self.view.show_error(error_from_exception(error, 'theme'))
            return
        self.view.hide_banner()
        self.view.set_theme_list()
        self.render_page(self.character_models() if self.state.route.area == 'themes' else None)
        self.update_runtime()

    def show_translation_conflict_banner(self):
        self.view.show_banner('Translation settings changed in another window.', [
            ('Use latest', lambda: self.resolve_translation('latest')),
            ('Keep mine', lambda: self.resolve_translation('mine')),
            ('Compare', lambda: self.resolve_translation('compare'))])

    def resolve_translation(self, action):
        if action == 'compare':
            self.show_translation_compare(self.controller.resolve_translation_conflict('compare'))
            return
        self.controller.resolve_translation_conflict(action)
        self.view.hide_banner()
        self.view.update_translation(self.controller.translation(self.state.draft.value['id']))
        self.update_runtime()

    def show_translation_compare(self, conflict):
        if not conflict:
            return
        dialog = Gtk.MessageDialog(transient_for=self.view, modal=False, buttons=Gtk.ButtonsType.CLOSE,
                                   text=_('Compare translation settings'))
        dialog.set_property('secondary-text',
                            (_('Mine') + ': ' + str(conflict['mine'])[:240] + '\n' +
                             _('Latest') + ': ' + str(conflict['latest'])[:240]))
        dialog.connect('response', lambda widget, *_: widget.destroy())
        dialog.present()

    def render_page(self, models=None):
        theme = self.state.draft.value
        self.view.set_theme_resources(self.controller.feature_read_models(),
                                      self.controller.bubble_preview_read_model(),
                                      self.controller.translation(theme['id']),
                                      dict(self.state.bubbles))
        self.view.library_snapshot = self.controller.resource_snapshot
        self.view.library_job = self.library_job
        self.view.render_page(models if models is not None else self.character_models())

    def character_models(self):
        return self.controller.character_read_models()

    def select_character(self, key):
        if key in self.state.draft.value['characters']:
            self.state.route.detail = key
            self.view.select_preview_character(key)

    def poll_status(self):
        if self.disposed:
            return False
        self.refresh_status()
        return True

    def navigate(self, area):
        if self.state.route.area == 'library' and area != 'library':
            self.library_generation += 1
            self.resource_runner.cancel_pending(JobKind.RESOURCE)
            if self.library_file_dialog:
                self.library_file_dialog.cancel()
                self.library_file_dialog = None
        self.controller.set_translation_editing(self.state.draft.value['id'] if area == 'themes' and self.state.route.tab == 'features' else None)
        self.state.route.area = area
        self.view.active_area = area
        self.render_page(self.character_models() if area == 'themes' else None)
        self.update_runtime()
        if area == 'library':
            self.refresh_resources()
        if area == 'connections' and not self.catalog_requested:
            self.refresh_models()

    def select_tab(self, tab):
        self.controller.set_translation_editing(self.state.draft.value['id'] if tab == 'features' else None)
        self.state.route.tab = tab
        self.view.active_tab = tab
        self.render_page(self.character_models() if self.state.route.area == 'themes' else None)
        self.update_runtime()

    def edit_cast(self, operation, key):
        if operation == 'locate':
            self.library_file('import', 'characters', key)
            return
        operations = {'add': lambda: self.controller.add_character(key),
                      'disable': lambda: self.controller.disable_character(key),
                      'primary': lambda: self.controller.set_primary(key),
                      'up': lambda: self.controller.move_character(key, -1),
                      'down': lambda: self.controller.move_character(key, 1),
                      'replace': lambda: self.controller.replace_character(*key),
                      'reorder': lambda: self.controller.reorder_character(*key)}
        if operation in operations:
            self.edits.edit(operations[operation], scope='cast')

    def offer_undo(self, *args):
        return self.edits.offer_undo(*args)

    def invalidate_undo(self, *args):
        return self.edits.invalidate_undo(*args)

    def expire_undo(self, *args):
        return self.edits.expire_undo(*args)

    def undo(self, *args):
        return self.edits.undo(*args)

    def edit_bubble(self, key):
        if key != self.state.draft.value['bubble']:
            self.edits.edit(lambda: self.controller.set_bubble(key))

    def edit_feature(self, key, enabled):
        review = self.controller.plugin_review(key) if enabled else None
        if review:
            self.confirm_plugin(review)
            return
        if (key in self.state.draft.value['plugins']) != enabled:
            self.edits.edit(lambda: self.controller.set_plugin_enabled(key, enabled), scope='features')

    def confirm_plugin(self, review, enable=True):
        if self.plugin_review_dialog:
            self.plugin_review_dialog.present()
            return
        from manager_plugin_review import review_details
        theme_id = self.state.draft.value['id']
        library_generation = self.library_generation
        dialog = Gtk.MessageDialog(transient_for=self.view, modal=True, buttons=Gtk.ButtonsType.NONE,
                                   text=_('Approve and enable plugin?' if enable else 'Approve plugin?'))
        dialog.set_property('secondary-text', review_details(review))
        dialog.add_button(_('Cancel'), Gtk.ResponseType.CANCEL)
        dialog.add_button(_('Approve and enable' if enable else 'Approve'), Gtk.ResponseType.ACCEPT)
        self.plugin_review_dialog = dialog
        self.render_page(self.character_models())
        def answered(widget, response):
            widget.destroy()
            self.plugin_review_dialog = None
            if response != Gtk.ResponseType.ACCEPT or self.disposed:
                return
            if not enable:
                if library_generation == self.library_generation and self.state.route.area == 'library':
                    self.run_resource('approve', 'plugins', key=review['id'], repair={'package_sha256': review['package_sha256']})
                return
            generation = self.generation
            def approved(result, error):
                if self.disposed or generation != self.generation:
                    return
                if error:
                    self.view.show_error(error)
                else:
                    self.state.plugins[review['id']] = result
                    if self.state.draft.value['id'] == theme_id:
                        self.edits.edit(lambda: self.controller.set_plugin_enabled(review['id'], True), scope='features')
                if self.state.commands.closing and not self.runner.active:
                    self.finish_close()
                else:
                    self.refresh_resources()
            self.resource_runner.submit(JobKind.CATALOG,
                lambda: self.controller.approve_plugin(review['id'], review['package_sha256']), approved)
        dialog.connect('response', answered)
        dialog.present()

    def library_category(self, category):
        if category not in ('characters', 'bubbles', 'plugins'):
            return
        if self.state.route.library_tab != category:
            self.library_generation += 1
            self.resource_runner.cancel_pending(JobKind.RESOURCE)
        self.state.route.library_tab = category
        self.render_page()

    def library_review(self, key):
        review = self.controller.plugin_review(key)
        if review:
            self.confirm_plugin(review, enable=False)

    def library_file(self, action, kind, key):
        if self.library_file_dialog:
            return
        generation = self.library_generation
        title = {'characters': 'Import Character', 'bubbles': 'Import Bubble', 'plugins': 'Import Plugin'}[kind]
        chooser = Gtk.FileDialog(title=_('Upgrade' if action == 'upgrade' else title))
        archive = Gtk.FileFilter()
        archive.set_name(_(title) + ' (*.zip)')
        for pattern in ('*.zip',):
            archive.add_pattern(pattern)
        filters = Gio.ListStore.new(Gtk.FileFilter)
        filters.append(archive)
        chooser.set_filters(filters)
        cancel = Gio.Cancellable()
        self.library_file_dialog = cancel
        def selected(dialog, result):
            try:
                file = dialog.open_finish(result)
            except GLib.Error as error:
                if (not cancel.is_cancelled() and error.code not in (Gtk.DialogError.DISMISSED, Gtk.DialogError.CANCELLED)
                        and not self.disposed and generation == self.library_generation):
                    self.view.show_error(error_from_exception(error, 'resource'))
                return
            finally:
                if self.library_file_dialog is cancel:
                    self.library_file_dialog = None
            if file and not self.disposed and generation == self.library_generation:
                self.run_resource(action, kind, path=file.get_path(), key=key)
        chooser.open(self.view, cancel, selected)

    def library_runtime(self, action, theme_id):
        if not self.mutation_presentation().runtime_allowed:
            self.view.show_notice("Waiting for resource transaction")
            return
        if self.disposed or not self.flush_save():
            return
        generation = self.generation
        def completed(result, error):
            if self.disposed or generation != self.generation:
                return
            if error:
                self.view.show_error(error)
            else:
                self.state.runtime.update(result)
                self.view.clear_error()
            self.update_runtime()
            if self.state.commands.closing and not self.runner.active and not self.runner.queue and not self.resource_runner.active and not self.resource_runner.queue:
                self.finish_close()
            else:
                self.refresh_resources()
        operation = self.controller.stop if action == 'stop' else lambda: self.controller.apply_saved_replacement(theme_id)
        self.runner.submit(JobKind.RUNTIME, operation, completed, self.update_runtime)
        self.update_runtime()

    def library_remove(self, kind, key, repair):
        self.run_resource('uninstall', kind, key=key, repair=repair)

    def cancel_resource_tasks(self):
        count = self.resource_runner.cancel_pending(JobKind.RESOURCE)
        if not self.disposed:
            self.view.show_notice(_('Cancelled %d queued tasks. Running transactions finish safely.') % count)

    def run_resource(self, action, kind, **arguments):
        if action != "import" and self.mutation_presentation().runtime_mutation_active:
            self.view.show_notice("Waiting for Runtime")
            return
        if self.disposed or not self.flush_save():
            return
        self.invalidate_undo()
        generation, page_generation = self.generation, self.library_generation
        def visible():
            return (not self.disposed and generation == self.generation and
                    page_generation == self.library_generation and self.state.route.area == 'library')
        self.resource_task_sequence += 1
        task_id = self.resource_task_sequence
        def set_progress(phase, error_code=None):
            task = ResourceTask(task_id, action, kind, phase, error_code)
            self.resource_tasks[task_id] = task
            if len(self.resource_tasks) > 128:
                del self.resource_tasks[next(iter(self.resource_tasks))]
            if self.disposed:
                return False
            current = self.library_job
            terminal = phase in ('completed', 'failed', 'cancelled')
            if terminal and current and current.id != task_id:
                return False
            self.library_job = None if terminal else task
            self.view.library_job = self.library_job
            if visible() and hasattr(self.view.page, 'update_job'):
                self.view.page.update_job(self.library_job)
            self.update_runtime()
            if terminal and visible():
                self.view.show_notice({'completed': 'Completed', 'failed': 'Failed', 'cancelled': 'Cancelled'}[phase])
            return False
        def progress(phase):
            GLib.idle_add(set_progress, phase)
        def started():
            waiting = action in ('upgrade', 'uninstall', 'finalize', 'cancel_removal') and self.mutation_presentation().runtime_mutation_active
            set_progress('queued' if waiting else 'validating')
        def completed(result, error):
            if self.disposed or generation != self.generation:
                return
            if error and error.code == 'resource.cancelled':
                set_progress('cancelled', error.code)
                return
            set_progress('failed' if error else 'completed', error.code if error else None)
            if visible():
                self.library_error = error
                if error:
                    self.view.show_error(error)
                    if error.code == 'theme.conflict':
                        self.view.show_banner('Theme updated in another window. Review dependencies again.', [
                            ('Reload latest', self.refresh_resources), ('Keep editing', self.view.hide_banner)])
                else:
                    self.view.clear_error()
                    self.view.hide_banner()
                    self.view.show_notice('Completed')
            # Global synchronization continues even if the originating page closed.
            # Only its presentation result/error is discarded.
            if self.state.commands.closing:
                if not self.runner.active and not self.runner.queue and not self.resource_runner.active and not self.resource_runner.queue:
                    self.finish_close()
                return
            self.refresh_resources()
            if not error:
                result = self.controller.sync_theme()
                if result == 'pending':
                    self.show_theme_update_banner()
                elif result == 'reloaded':
                    self.view.set_theme_list()
        if self.library_job is None:
            set_progress('queued')
        else:
            self.resource_tasks[task_id] = ResourceTask(task_id, action, kind, 'queued')
        self.resource_runner.submit(JobKind.RESOURCE,
            lambda: self.controller.resource_operation(action, kind, progress=progress, **arguments), completed, started)

    def edit_translation(self, field, value):
        if self.disposed or field not in ('target', 'model', 'fast'):
            return
        try:
            self.controller.edit_translation(self.state.draft.value['id'], **{field: value})
        except AppError as error:
            self.view.show_error(error)
            if error.code == 'translation.conflict':
                self.show_translation_conflict_banner()
            return
        self.view.clear_error()
        self.view.update_translation(self.controller.translation(self.state.draft.value['id']))
        self.update_runtime()

    def edit_behavior(self, field, value):
        if field in ('size', 'idle_after') and self.state.draft.value[field] != value:
            self.edits.edit(lambda: self.controller.edit_theme(**{field: value}), deferred=True)

    def choose(self, key):
        if key == self.state.draft.value['id']:
            return
        if not self.flush_save():
            return
        try:
            self.controller.choose(key)
        except AppError as error:
            self.view.show_error(error)
            return
        self.invalidate_undo()
        self.controller.set_translation_editing(key if self.state.route.tab == 'features' else None)
        self.view.clear_error()
        self.view.set_theme_list()
        self.navigate('themes')

    def create_theme(self, name):
        if not self.flush_save() or self.disposed:
            return
        self.controller.create(name)
        self.invalidate_undo()
        self.view.set_theme_list()
        self.navigate('themes')

    def import_theme(self, path):
        if not self.flush_save() or self.disposed:
            return
        result = self.controller.import_file('theme', path)
        self.invalidate_undo()
        self.controller.after_import()
        self.view.set_theme_list()
        self.navigate('themes')
        return result

    def edit_name(self, name):
        if name != self.state.draft.value['name']:
            self.edits.edit(lambda: self.controller.edit_theme(name=name), deferred=True)



    def flush_save(self):
        return self.edits.flush()

    def show_conflict(self):
        if self.conflict_dialog:
            self.conflict_dialog.present()
            return
        dialog = Gtk.MessageDialog(transient_for=self.view, modal=True,
                                   buttons=Gtk.ButtonsType.NONE,
                                   text=_('Theme changed in another window'))
        dialog.set_property('secondary-text', _('Your edits are still here. Choose how to resolve the newer saved version.'))
        dialog.add_button(_('Keep editing'), Gtk.ResponseType.CANCEL)
        dialog.add_button(_('Reload latest'), Gtk.ResponseType.NO)
        dialog.add_button(_('Save as new Theme'), Gtk.ResponseType.ACCEPT)
        self.conflict_dialog = dialog
        def answered(widget, response):
            widget.destroy()
            self.conflict_dialog = None
            if response == Gtk.ResponseType.CANCEL:
                return
            try:
                if response == Gtk.ResponseType.NO:
                    self.invalidate_undo()
                    self.controller.reload_latest()
                elif response == Gtk.ResponseType.ACCEPT:
                    self.invalidate_undo()
                    self.controller.save_as_new()
                self.view.clear_error()
                self.view.set_theme_list()
                self.navigate('themes')
            except (OSError, ValueError) as failure:
                self.view.show_error(error_from_exception(failure, 'theme'))
        dialog.connect('response', answered)
        dialog.present()

    def mutation_presentation(self):
        return project_mutations(self.runner.active.kind if self.runner.active else None, self.library_job)

    def runtime_info(self):
        theme = self.state.draft.value
        projection = project_runtime(self.state.runtime, theme, self.controller.translation(theme['id']),
                                     self.controller.same_config)
        pending = ('Draft needs saving' if self.state.draft.needs_save else
                   'Saved changes await Apply' if projection.dirty else
                   'Theme is running' if projection.applied else '')
        action = ('Retry connection' if self.state.runtime.disconnected and self.state.runtime.snapshot is None
                  else projection.action)
        status = ('Finishing resource task…' if self.resource_runner.active else 'Closing…') if self.state.commands.closing else projection.status
        missing = self.controller.missing_resource_models()
        return {'status': status, 'pending': pending, 'action': action,
                'repairs': [{'kind': item.kind, 'technical_id': item.technical_id, 'label': item.label} for item in missing],
                'collapsed': self.state.route.area in ('connections', 'settings') and not (self.state.draft.needs_save or projection.dirty),
                'busy': bool((missing and action != 'Retry connection') or self.runner.active or self.runner.queue or self.state.commands.closing or not self.mutation_presentation().runtime_allowed)}

    def update_runtime(self):
        if not self.disposed:
            self.view.update_runtime(self.runtime_info())
            for method in ('update_connections', 'update_models'):
                if hasattr(self.view.page, method):
                    getattr(self.view.page, method)()
            if hasattr(self.view.page, "update_mutations"):
                self.view.page.update_mutations(self.mutation_presentation())

    def repair_resources(self, kind):
        self.navigate('themes')
        self.select_tab({'characters': 'cast', 'bubbles': 'bubble', 'plugins': 'features'}[kind])

    def primary_action(self):
        if self.state.runtime.disconnected and self.state.runtime.snapshot is None:
            self.refresh_status()
        else:
            self.apply()

    def refresh_status(self):
        if self.disposed:
            return
        generation = self.generation
        def completed(result, error):
            if self.disposed or generation != self.generation:
                return
            if not error:
                result, connection, error = result
                self.connection_event(connection)
            if error:
                self.state.runtime.failed(error.code)
                self.view.show_error(error)
            else:
                was_disconnected = self.state.runtime.disconnected
                changed = self.state.runtime.snapshot != result
                self.state.runtime.update(result)
                if changed and self.state.route.area == 'library':
                    self.refresh_resources()
                if was_disconnected:
                    self.view.clear_error()
            self.update_runtime()
            if self.state.commands.closing and not self.runner.active and not self.runner.queue and not self.resource_runner.active and not self.resource_runner.queue:
                self.finish_close()
        self.runner.submit(JobKind.STATUS, self.controller.manager_status, completed)

    def apply(self):
        if not self.mutation_presentation().runtime_allowed:
            self.view.show_notice("Waiting for resource transaction")
            return
        if self.state.runtime.disconnected and self.state.runtime.snapshot is None:
            return
        if not self.flush_save() or self.disposed:
            return
        snapshot = self.state.draft.snapshot()
        options = self.controller.translation(snapshot['id'])
        generation = self.generation
        def completed(result, error):
            if self.disposed or generation != self.generation:
                return
            if error:
                self.state.runtime.failed(error.code)
                self.view.show_error(error)
            else:
                self.state.runtime.update(result)
                self.view.clear_error()
                try:
                    self.controller.mark_translation_applied(snapshot['id'], options)
                except (OSError, ValueError) as failure:
                    self.view.show_error(error_from_exception(failure, 'translation'))
            self.update_runtime()
            if self.state.commands.closing and not self.runner.active and not self.runner.queue and not self.resource_runner.active and not self.resource_runner.queue:
                self.finish_close()
        self.runner.submit(JobKind.RUNTIME, lambda: self.controller.apply(snapshot, options), completed,
                           self.update_runtime)
        self.update_runtime()

    def close(self, *_args):
        if self.disposed:
            return False
        if not self.flush_save():
            if self.conflict_dialog:
                self.conflict_dialog.present()
                return True
            if self.close_dialog:
                self.close_dialog.present()
                return True
            dialog = Gtk.MessageDialog(transient_for=self.view, modal=True,
                                       buttons=Gtk.ButtonsType.NONE,
                                       text=_('The draft could not be saved'))
            dialog.add_button(_('Keep editing'), Gtk.ResponseType.CANCEL)
            dialog.add_button(_('Discard draft and close'), Gtk.ResponseType.ACCEPT)
            self.close_dialog = dialog
            def answered(widget, response):
                widget.destroy()
                self.close_dialog = None
                if response == Gtk.ResponseType.ACCEPT:
                    self.dispose()
            dialog.connect('response', answered)
            dialog.present()
            return True
        if self.runner.active or self.runner.queue or self.resource_runner.active or self.resource_runner.queue:
            self.state.commands.closing = True
            self.resource_runner.state.closing = True
            self.runner.cancel_pending()
            self.resource_runner.cancel_pending()
            self.view.runtime_status.set_text('Closing when task finishes…')
            self.update_runtime()
            return True
        self.finish_close()
        return True

    def finish_close(self):
        if self.state.settings.get('close') == 'quit':
            self.state.commands.closing = False
            def completed(_result, error):
                if error:
                    self.state.commands.closing = False
                    self.resource_runner.state.closing = False
                    self.view.show_error(error)
                    self.update_runtime()
                else:
                    self.dispose()
            self.runner.submit(JobKind.RUNTIME, self.controller.stop, completed, self.update_runtime)
            self.state.commands.closing = True
        else:
            self.dispose()

    def dispose(self):
        if self.disposed:
            return
        self.disposed = True
        self.generation += 1
        self.edits.dispose()
        self.config_sync.close()
        self.session_subscription.close()
        self.invalidate_undo()
        GLib.source_remove(self.status_timer)
        if self.close_dialog:
            self.close_dialog.destroy()
            self.close_dialog = None
        if self.conflict_dialog:
            self.conflict_dialog.destroy()
            self.conflict_dialog = None
        if self.library_file_dialog:
            self.library_file_dialog.cancel()
            self.library_file_dialog = None
        self.resource_runner.shutdown()
        self.runner.shutdown()
        if self.plugin_review_dialog:
            self.plugin_review_dialog.destroy()
            self.plugin_review_dialog = None
        if self.catalog_job is not None:
            self.controller.stop_model_catalog(self.catalog_job)
            self.catalog_job = None
        for dialog in (self.settings_dialog, self.diagnostics_dialog):
            if dialog:
                dialog.destroy()
        self.controller.shutdown()
        self.view.dispose_style()
        self.view.destroy()


def main():
    GLib.set_prgname('io.mochi.ManagerV2')
    GLib.set_application_name('Mochi Manager v2')
    flags = Gio.ApplicationFlags.NON_UNIQUE if os.environ.get('MOCHI_MANAGER_V2_NON_UNIQUE') == '1' else Gio.ApplicationFlags.DEFAULT_FLAGS
    app = Gtk.Application(application_id='io.mochi.ManagerV2', flags=flags)
    presenters = []
    startup = ControllerBootstrap(lambda callback, *args: GLib.idle_add(callback, *args))
    opening = False
    def activate(application):
        nonlocal opening
        if presenters and not presenters[0].disposed:
            presenters[0].view.present()
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
                    dialog = Gtk.MessageDialog(buttons=Gtk.ButtonsType.CLOSE, text=_('Could not open Mochi'))
                    dialog.set_property('secondary-text', str(error)[:8192])
                    dialog.set_application(application)
                    dialog.connect('response', lambda widget, *_: widget.destroy())
                    dialog.present()
                    return False
                presenter = ManagerV2(application, controller)
                presenters[:] = [presenter]
                presenter.view.present()
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
