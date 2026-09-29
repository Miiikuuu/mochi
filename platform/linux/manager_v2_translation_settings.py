"""Translation draft controls; receives values and emits edits."""
from manager_v2_common import Gtk, Pango, styled, text
from manager_i18n import _
from manager_connections import ModelCatalogState


class TranslationSettings:
    def __init__(self, options, layout, callbacks, catalog=None):
        self.translation_options, self.layout, self.callbacks = options, layout, callbacks
        self.catalog = catalog or ModelCatalogState()
        self._setting_translation = False
        self.disposed = False
        self.build()

    def dispose(self):
        self.disposed = True
        self.callbacks = {'translation': lambda *_: None}

    def build(self):
        options = self.translation_options
        group = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['md']), 'translation-settings')
        group.append(text('Translation settings', 'field-label'))
        languages = [('zh-CN', 'Chinese (Simplified)'), ('en', 'English'), ('ja', 'Japanese'),
                     ('ko', 'Korean'), ('fr', 'French'), ('de', 'German'), ('es', 'Spanish')]
        target = Gtk.DropDown.new_from_strings([_(name) for _key, name in languages])
        target.set_selected(next((i for i, (key, _name) in enumerate(languages) if key == options['target']), 0))
        target.update_property([Gtk.AccessibleProperty.LABEL], [_('Target language')])
        target.connect('notify::selected', lambda widget, _param:
                       self.callbacks['translation']('target', languages[widget.get_selected()][0])
                       if not self._setting_translation else None)
        group.append(text('Target language', 'field-label'))
        group.append(target)
        model = Gtk.DropDown.new_from_strings([])
        model.set_hexpand(True)
        factory = Gtk.SignalListItemFactory()
        def setup(_factory, item):
            label = Gtk.Label(xalign=0)
            label.set_ellipsize(Pango.EllipsizeMode.END)
            label.set_max_width_chars(22)
            item.set_child(label)
        factory.connect('setup', setup)
        factory.connect('bind', lambda _factory, item: item.get_child().set_text(item.get_item().get_string()))
        model.set_factory(factory)
        model.set_list_factory(factory)
        model.update_property([Gtk.AccessibleProperty.LABEL], [_('Model')])
        model.connect('notify::selected', self._select_model)
        group.append(text('Model', 'field-label'))
        group.append(model)
        self.model_status = text('', 'section-hint')
        group.append(self.model_status)
        self.refresh = Gtk.Button(label=_('Refresh models'))
        self.refresh.connect('clicked', lambda _button: self.callbacks.get('models_refresh', lambda: None)())
        group.append(self.refresh)
        custom = Gtk.Entry(text=options['model'])
        custom.set_max_length(128)
        custom.set_hexpand(True)
        custom.update_property([Gtk.AccessibleProperty.LABEL], [_('Model ID')])
        custom.connect('activate', lambda widget: self._commit_model(widget))
        focus = Gtk.EventControllerFocus.new()
        focus.connect('leave', lambda _controller: self._commit_model(custom))
        custom.add_controller(focus)
        recovery = Gtk.Expander(label=_('Recover a model ID'))
        recovery.set_focusable(True)
        recovery.set_child(custom)
        group.append(recovery)
        fast_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['md'])
        fast_label = text('Fast', 'field-label')
        fast_label.set_hexpand(True)
        fast_row.append(fast_label)
        fast = Gtk.Switch(active=options['fast'])
        fast.update_property([Gtk.AccessibleProperty.LABEL], [_('Fast')])
        fast.connect('notify::active', lambda widget, _param:
                     self.callbacks['translation']('fast', widget.get_active())
                     if not self._setting_translation else None)
        fast_row.append(fast)
        group.append(fast_row)
        self.root = group
        self.translation_controls = {'target': target, 'model': model, 'custom_model': custom, 'fast': fast, 'languages': languages, 'recovery': recovery}
        self.update_catalog()


    def _select_model(self, widget, _param):
        index = widget.get_selected()
        if not self._setting_translation and index < len(self.model_ids):
            self.callbacks['translation']('model', self.model_ids[index])

    def update_catalog(self):
        if self.disposed:
            return
        self._setting_translation = True
        try:
            current = self.translation_options['model']
            self.model_ids = list(self.catalog.models)
            if current not in self.model_ids:
                self.model_ids.append(current)
            labels = [self.catalog.models.get(key, {}).get('name', key) for key in self.model_ids]
            self.translation_controls['model'].set_model(Gtk.StringList.new(labels))
            self.translation_controls['model'].set_selected(self.model_ids.index(current))
            spec = self.catalog.models.get(current)
            fast = spec.get('fast_tier') if spec else None
            label = 'Loading…' if self.catalog.loading else 'Unavailable' if spec is None else 'Fast supported' if fast else 'Fast unavailable'
            self.model_status.set_text(_(label) + (' — '+_('Model refresh failed') if self.catalog.error else ''))
            self.refresh.set_sensitive(not self.catalog.loading)
            # Unknown historical IDs are retained, including their saved Fast choice.
            self.translation_controls['fast'].set_sensitive(spec is None or bool(fast) or self.translation_options['fast'])
        finally:
            self._setting_translation = False

    def _commit_model(self, entry):
        value = entry.get_text().strip()
        if value and value != self.translation_options['model']:
            self.callbacks['translation']('model', value)


    def update_translation(self, options):
        self.translation_options = options
        controls = self.translation_controls
        if not controls:
            return
        self._setting_translation = True
        try:
            index = next((i for i, (key, _name) in enumerate(controls['languages']) if key == options['target']), 0)
            controls['target'].set_selected(index)
            if controls['custom_model'].get_text() != options['model']:
                controls['custom_model'].set_text(options['model'])
            controls['fast'].set_active(options['fast'])
            self.update_catalog()
        finally:
            self._setting_translation = False
