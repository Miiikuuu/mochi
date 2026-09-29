"""Bundled translation adapter behind the same lifecycle as imported plugins.

Legacy browser/shortcut requests remain here, outside the presentation host.
"""
import json
import os
from pathlib import Path
from native_jobs import NativeJobs
from translation_controller import TranslationController, validate_text
from translation_panel import TranslationPanel, contents
from theme_store import atomic_json
from manager_preferences import translation_options


class TranslationPlugin:
    request_types = ('translate', 'translation_settings')

    def __init__(self, host, spec):
        self.host, self.spec = host, spec
        self.key = spec['id']
        self.enabled = False
        self.transport = NativeJobs(os.environ.get('MOCHI_TRANSLATION_LAUNCHER', str(spec['root']/'.local/native-host')))
        prefs = host.store.directory/'translation.json'
        if host.theme_config:
            scoped = host.store.directory/'themes'/host.theme_config['id']/'translation.json'
            if not scoped.exists() and prefs.exists():
                try:
                    atomic_json(scoped, json.loads(prefs.read_text()))
                except (OSError, ValueError):
                    pass
            prefs = scoped
        self.controller = TranslationController(host.messages, self.transport, prefs, host.copy_text, lambda: host.open_function(self.key))
        self.panel = TranslationPanel(self.controller, host.close_panel, host.follow_once, host.themes, host.select_theme)
        if os.environ.get('MOCHI_THEME_TRANSLATION'):
            self.configure_legacy(json.loads(os.environ['MOCHI_THEME_TRANSLATION']))

    @property
    def busy(self):
        return self.controller.busy

    def configure(self, theme):
        if theme:
            path = self.host.store.directory/'themes'/theme['id']/'translation.json'
            if path != self.controller.preferences_path:
                atomic_json(path, self.controller.preferences)
                self.controller.preferences_path = path

    def configure_legacy(self, options):
        if options is not None:
            options = translation_options(options)
            atomic_json(self.controller.preferences_path, options)
            self.controller.preferences = options
            self.controller.changed()

    def open(self):
        self.host.show_panel(self)
        from gi.repository import Gtk
        self.panel.window.present_with_time(Gtk.get_current_event_time())

    def close_panel(self):
        self.panel.window.hide()
        self.controller.stop_catalog()
        self.controller.dismiss()

    def stop(self):
        self.close_panel()
        for channel in list(self.transport.channels.values()):
            self.transport.close_channel(channel)

    def shutdown(self):
        self.stop()
        self.transport.shutdown()
        self.panel.window.destroy()

    def status_extra(self):
        return {'translation': dict(self.controller.preferences)}

    def request(self, request):
        if request['type'] == 'translation_settings':
            self.open()
            self.panel.open_settings()
            return {'accepted': True}
        text = request.get('text', '')
        validate_text(text)
        options = {key: request.get(key) for key in ('target', 'model', 'fast')}
        self.controller.validate_options(**options)
        presentation = request.get('presentation', 'speech' if text.strip() else 'panel')
        if presentation not in ('speech', 'panel'):
            raise ValueError('不支持的翻译展示方式。')
        if not text.strip() and all(value is None for value in options.values()):
            self.open()
            return {'accepted': True}
        if any(value is not None for value in options.values()):
            self.controller.configure(**options)
        if text.strip():
            self.controller.start(text)
        else:
            self.controller.dismiss()
        self.panel.original.set_expanded(not bool(text))
        if presentation == 'panel':
            self.host.show_panel(self)
        else:
            self.host.show('speech')
        return {'accepted': True}

    def inspect(self):
        c = self.controller
        return {'busy': c.busy, 'text': c.text, 'source': c.source,
                'model': c.actual_model, 'status': c.status,
                'worker_pid': c.job['proc'].pid if c.job else None,
                'selected_model': c.preferences['model'], 'selected_target': c.preferences['target'],
                'fast': c.preferences['fast'], 'fast_sensitive': self.panel.fast.get_sensitive(),
                'settings_visible': self.panel.stack.get_visible_child_name() == 'settings',
                'catalog_busy': c.catalog_job is not None, 'models': list(c.catalog or {}),
                'catalog_status': c.catalog_status, 'message_id': c.message_id,
                'panel_text': contents(self.panel.result)}
