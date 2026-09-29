"""Library presentation over ResourceCatalogSnapshot; no resource services or IO."""
import json
from gi.repository import Gio, GObject
from manager_v2_common import Gdk, GLib, Gtk, Pango, styled, text, clear, thumbnail_pixbuf
from manager_i18n import _
from manager_mutation_presentation import MutationPresentation


class ResourceCard(Gtk.Box):
    """Keep Tab inside the currently focused card before leaving the grid item."""
    def __init__(self, spacing):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=spacing)
        self.focus_order = []

    def do_focus(self, direction):
        if direction not in (Gtk.DirectionType.TAB_FORWARD, Gtk.DirectionType.TAB_BACKWARD):
            return Gtk.Box.do_focus(self, direction)
        focus = self.get_root().get_focus() if self.get_root() else None
        targets = [item for item in self.focus_order if item.get_mapped() and item.get_sensitive()]
        backwards = direction == Gtk.DirectionType.TAB_BACKWARD
        if backwards:
            targets.reverse()
        for index, item in enumerate(targets):
            if focus is item or focus and focus.is_ancestor(item):
                return targets[index+1].grab_focus() if index+1 < len(targets) else False
        return targets[0].grab_focus() if targets else False


class ResourceItem(GObject.Object):
    def __init__(self, key, record):
        super().__init__()
        self.key, self.record = key, record


class LibraryPage:
    def __init__(self, snapshot, category, callbacks, layout, compact, job):
        self.snapshot, self.category = snapshot, category
        self.layout, self.compact = layout, compact
        self.disposed = False
        self.callbacks = {key: (lambda *args, operation=value: operation(*args) if not self.disposed else None)
                          for key, value in callbacks.items()}
        self.root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=layout['spacing']['md'])
        self.root.set_vexpand(True)
        self.root.append(text('Library', 'page-title'))
        self.categories, self.cards, self.details, self.approvals, self.removals = {}, {}, {}, {}, {}
        self.overflows = {}
        self.mutations = MutationPresentation()
        self.mutation_buttons = {}
        self.execution_buttons = []
        self.confirm_button = None
        self.slots = {}
        self.visible_tick = None
        self.pending_scroll = None
        categories = Gtk.Box(spacing=layout['spacing']['sm'])
        for key, title in (('characters', 'Characters'), ('bubbles', 'Bubbles'), ('plugins', 'Plugins')):
            button = Gtk.Button(label=_(title))
            if key == category:
                button.add_css_class('active')
            button.connect('clicked', lambda _button, value=key: self.callbacks['library_category'](value))
            categories.append(button)
            self.categories[key] = button
        self.root.append(categories)
        self.context = Gtk.DropDown.new_from_strings([_(name) for name in ('All', 'Needs review', 'In use', 'Unused', 'Updates')])
        self.context.set_tooltip_text(_('Resource filter'))
        self.search = Gtk.SearchEntry(placeholder_text=_('Search resources'))
        self.search.set_hexpand(True)
        self.sort = Gtk.DropDown.new_from_strings([_('Name A–Z'), _('Name Z–A'), _('Most used')])
        self.sort.set_tooltip_text(_('Sort resources'))
        self.toolbar = Gtk.Box(spacing=layout['spacing']['sm'])
        self.filter_row = Gtk.Box(spacing=layout['spacing']['sm'])
        self.context.set_hexpand(True)
        self.sort.set_hexpand(True)
        for widget in (self.context, self.sort):
            self.filter_row.append(widget)
        self.toolbar.append(self.search)
        self.toolbar.append(self.filter_row)
        self.root.append(self.toolbar)
        self.import_button = Gtk.Button(label=_({'characters': 'Import Character', 'bubbles': 'Import Bubble', 'plugins': 'Import Plugin'}[category]))
        self.import_button.set_halign(Gtk.Align.START)
        self.import_button.connect('clicked', lambda *_: self.callbacks['library_file']('import', category, None))
        self.root.append(self.import_button)
        self.progress = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=layout['spacing']['sm'])
        self.progress_label = text('')
        self.progress_label.set_wrap(True)
        self.progress.append(self.progress_label)
        self.cancel_button = Gtk.Button(label=_('Cancel queued tasks'))
        self.cancel_button.connect('clicked', lambda *_: self.callbacks['library_cancel']())
        self.progress.append(self.cancel_button)
        self.root.append(self.progress)
        self.update_job(job)
        self.content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=layout['spacing']['md'])
        self.content.set_vexpand(True)
        self.root.append(self.content)
        self.model = Gio.ListStore.new(ResourceItem)
        self.selection = Gtk.NoSelection.new(self.model)
        self.factory = Gtk.SignalListItemFactory()
        self.factory.connect('bind', self._bind)
        self.factory.connect('unbind', self._unbind)
        self.grid = Gtk.GridView.new(self.selection, self.factory)
        self.grid.set_single_click_activate(False)
        self.grid_scroll = Gtk.ScrolledWindow()
        self.grid_scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.grid_scroll.set_vexpand(True)
        self.grid_scroll.set_child(self.grid)
        self.content.append(self.grid_scroll)
        adjustment = self.grid_scroll.get_vadjustment()
        adjustment.connect('value-changed', lambda *_: self.schedule_visible())
        adjustment.connect('changed', lambda *_: self.schedule_visible())
        self.empty = text('No matching resources.', 'section-hint')
        self.content.append(self.empty)
        self.search.connect('search-changed', lambda *_: self.filter_items())
        self.sort.connect('notify::selected', lambda *_: self.filter_items())
        self.context.connect('notify::selected', lambda *_: self.filter_items())
        self.drop_target = Gtk.DropTarget.new(Gdk.FileList, Gdk.DragAction.COPY)
        self.drop_target.connect('drop', self._drop)
        self.root.add_controller(self.drop_target)
        self.filter_items()
        self.responsive(compact)

    def _drop(self, _target, value, _x, _y):
        files = value.get_files()
        if not files or len(files) > 16 or any(file.get_path() is None for file in files):
            return False
        for file in files:
            self.callbacks['library_drop'](self.category, file.get_path())
        return True

    def filter_items(self):
        if self.disposed:
            return
        query = self.search.get_text().casefold().strip()
        choice = self.context.get_selected()
        entries = getattr(self.snapshot, self.category) if self.snapshot else {}
        dependencies = self.snapshot.dependencies.get(self.category, {}) if self.snapshot else {}
        runtime = self.snapshot.runtime_dependencies if self.snapshot else ()
        def used(key):
            return bool(dependencies.get(key)) or any(item.uses(self.category, key) for item in runtime)
        def matches(key, record):
            if query and query not in ' '.join(str(record.get(field, '')) for field in ('display_name', 'name', 'description')).casefold() and query not in key.casefold():
                return False
            return (choice == 0 or choice == 1 and record.get('review') == 'unreviewed' or
                    choice == 2 and used(key) or choice == 3 and not used(key) or
                    choice == 4 and bool(record.get('update_available')))
        items = [ResourceItem(key, record) for key, record in entries.items() if matches(key, record)]
        name = lambda item: str(item.record.get('display_name', item.record.get('name', item.key))).casefold()
        sort = self.sort.get_selected()
        items.sort(key=(lambda item: (-len(dependencies.get(item.key, [])), name(item))) if sort == 2 else name,
                   reverse=sort == 1)
        self.model.splice(0, self.model.get_n_items(), items)
        self.empty.set_text(_('No updates reported by this catalog.' if choice == 4 else 'No matching resources.'))
        self.empty.set_visible(not items)

    def view_state(self):
        return {'query': self.search.get_text(), 'filter': self.context.get_selected(),
                'sort': self.sort.get_selected(), 'scroll': self.grid_scroll.get_vadjustment().get_value()}

    def restore_state(self, state):
        self.search.set_text(state.get('query', ''))
        self.context.set_selected(state.get('filter', 0))
        self.sort.set_selected(state.get('sort', 0))
        self.filter_items()
        self.pending_scroll = state.get('scroll', 0)
        self.schedule_visible()

    def schedule_visible(self):
        if not self.disposed and self.visible_tick is None:
            self.visible_tick = self.grid.add_tick_callback(self.refresh_visible)

    def refresh_visible(self, *_args):
        self.visible_tick = None
        if self.disposed:
            return False
        if self.pending_scroll is not None and self.grid_scroll.get_height() > 0:
            self.grid_scroll.get_vadjustment().set_value(self.pending_scroll)
            self.pending_scroll = None
        overscan = self.layout['library']['cardHeight'][self.category] * self.layout['library']['overscanRows']
        height = self.grid_scroll.get_height()
        focus = self.root.get_root().get_focus() if self.root.get_root() else None
        changed = False
        for key, slot in tuple(self.slots.items()):
            valid, bounds = slot.compute_bounds(self.grid_scroll)
            visible = valid and slot.get_mapped() and slot.get_width() > 0 and bounds.get_y() < height+overscan and bounds.get_y()+bounds.get_height() > -overscan
            child = slot.get_first_child()
            focused = child and focus and (focus is child or focus.is_ancestor(child))
            if visible and child is None:
                slot.append(self._card(key, slot.record))
                changed = True
            elif not visible and child is not None and not focused:
                slot.remove(child)
                self.forget_card(key)
                changed = True
        if changed:
            self.schedule_visible()
        return False

    def _bind(self, _factory, item):
        resource = item.get_item()
        item.set_focusable(True)
        item.set_activatable(False)
        item.resource_key = resource.key
        slot = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        slot.set_size_request(-1, self.layout['library']['cardHeight'][self.category])
        slot.record = resource.record
        self.forget_card(resource.key)
        self.slots[resource.key] = slot
        item.set_child(slot)
        self.schedule_visible()

    def forget_card(self, key):
        for entries in (self.cards, self.details, self.approvals, self.removals, self.overflows, self.mutation_buttons):
            entries.pop(key, None)

    def _unbind(self, _factory, item):
        key = getattr(item, 'resource_key', None)
        if self.slots.get(key) is item.get_child():
            self.slots.pop(key, None)
            self.forget_card(key)
        item.set_child(None)

    def scroll_to(self, key):
        self.pending_scroll = None
        for position in range(self.model.get_n_items()):
            if self.model.get_item(position).key == key:
                self.grid.scroll_to(position, Gtk.ListScrollFlags.FOCUS, None)
                self.schedule_visible()
                return True
        return False

    def _label(self, value):
        label = text(value, 'feature-meta')
        label.set_wrap(True)
        label.set_wrap_mode(Pango.WrapMode.WORD_CHAR)
        label.set_max_width_chars(24)
        return label

    def _card(self, key, record):
        card = styled(ResourceCard(self.layout['spacing']['sm']), 'feature-card')
        heading = Gtk.Box(spacing=self.layout['spacing']['md'])
        thumbnail = self.snapshot.thumbnails.get(key) if self.category == 'characters' else None
        if thumbnail:
            picture = Gtk.Picture.new_for_paintable(Gdk.Texture.new_for_pixbuf(thumbnail_pixbuf(thumbnail)))
            picture.set_content_fit(Gtk.ContentFit.CONTAIN)
            picture.set_size_request(self.layout['brandSize'], self.layout['brandSize'])
            picture.set_can_shrink(True)
            heading.append(picture)
        name = text(record.get('display_name', record.get('name', key)), 'theme-row-name')
        name.set_ellipsize(Pango.EllipsizeMode.END)
        name.set_hexpand(True)
        name.set_tooltip_text(name.get_text())
        heading.append(name)
        card.append(heading)
        lifecycle = record.get('lifecycle', 'active')
        if lifecycle != 'active':
            if self.category in ('characters', 'bubbles'):
                card.append(self._label('Replace this resource in saved Themes before applying.'))
            card.append(self._label('Pending removal' if lifecycle == 'pending_removal' else 'Missing'))
        if record.get('adoptable'):
            adopt = Gtk.Button(label=_('Adopt into Library'))
            adopt.connect('clicked', lambda *_: self.callbacks['library_lifecycle']('adopt', self.category, key))
            card.append(adopt)
            card.focus_order.append(adopt)
        if any(item.uses(self.category, key) for item in self.snapshot.runtime_dependencies):
            card.append(self._label('Running'))
        users = self.snapshot.dependencies.get(self.category, {}).get(key, [])
        card.append(self._label(_(record.get('origin', 'local'))))
        usage = Gtk.Button(label=(_('%d Saved Theme') if len(users) == 1 else _('%d Saved Themes')) % len(users))
        usage.connect('clicked', lambda *_: self.dependency_details(key))
        card.append(usage)
        card.focus_order.append(usage)
        if record.get('description'):
            description = self._label(record['description'])
            description.set_lines(2)
            description.set_ellipsize(Pango.EllipsizeMode.END)
            card.append(description)
        if self.category == 'bubbles':
            preview = self.snapshot.previews.get(key)
            sample = self._label('Hello from Mochi')
            sample.add_css_class('library-bubble-sample')
            if preview:
                provider = Gtk.CssProvider()
                provider.load_from_data(('.library-bubble-sample { background: '+preview.background+
                    '; color: '+preview.foreground+'; border-color: '+preview.border+'; }').encode())
                sample.get_style_context().add_provider(provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION+2)
            card.append(sample)
        if self.category == 'plugins':
            for value in (record.get('version', '') + ' · ' + record.get('runtime', ''),
                          ', '.join(record.get('capabilities', [])) or _('No declared capabilities'),
                          _(record.get('review', 'unreviewed')) + ' · ' + _(record.get('isolation', 'unsandboxed'))):
                card.append(self._label(value))
            digest = self._label('SHA-256: ' + record.get('package_sha256', '')[:10])
            digest.set_selectable(True)
            digest_row = Gtk.Box(spacing=self.layout['spacing']['sm'])
            digest.set_hexpand(True)
            digest_row.append(digest)
            copy = Gtk.Button(icon_name='edit-copy-symbolic')
            copy.set_tooltip_text(_('Copy SHA-256'))
            copy.connect('clicked', lambda *_: Gdk.Display.get_default().get_clipboard().set(record.get('package_sha256', '')))
            digest_row.append(copy)
            card.append(digest_row)
            card.focus_order.append(copy)
            if lifecycle == 'active' and record.get('origin') == 'local' and record.get('review') != 'approved':
                approve = Gtk.Button(label=_('Review / Approve'))
                approve.connect('clicked', lambda *_: self.callbacks['library_review'](key))
                card.append(approve)
                card.focus_order.append(approve)
                self.approvals[key] = approve
        details = Gtk.ToggleButton(label=_('Security Details' if self.category == 'plugins' else 'Reveal metadata'))
        details.set_focusable(True)
        metadata = Gtk.TextView(editable=False, cursor_visible=False, wrap_mode=Gtk.WrapMode.WORD_CHAR)
        safe = {'id': key, **{field: value for field, value in record.items() if field not in ('root', 'path', 'preview')}}
        metadata.get_buffer().set_text(json.dumps(safe, ensure_ascii=False, indent=2, default=str)[:8192])
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_size_request(-1, self.layout['brandSize']*3)
        scroll.set_child(metadata)
        scroll.set_visible(False)
        details.connect('toggled', lambda button: scroll.set_visible(button.get_active()))
        detail_actions = Gtk.Box(spacing=self.layout['spacing']['sm'])
        details.set_hexpand(True)
        detail_actions.append(details)
        card.append(detail_actions)
        card.focus_order.append(details)
        card.append(scroll)
        card.focus_order.append(metadata)
        self.details[key] = details
        if record.get('origin') == 'local':
            actions = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['sm'])
            upgrade = Gtk.Button(label=_('Upgrade'))
            upgrade.connect('clicked', lambda *_: self.callbacks['library_file']('upgrade', self.category, key))
            upgrade.lifecycle_allowed = lifecycle == 'active'
            upgrade.set_sensitive(upgrade.lifecycle_allowed and self.mutations.resource_allowed)
            actions.append(upgrade)
            remove = Gtk.Button(label=_('Uninstall…'))
            remove.connect('clicked', lambda *_: self.confirm_removal(key))
            actions.append(remove)
            self.removals[key] = remove
            overflow = Gtk.MenuButton(icon_name='view-more-symbolic')
            overflow.set_tooltip_text(_('Resource actions'))
            popover = Gtk.Popover()
            popover.set_child(actions)
            overflow.set_popover(popover)
            heading.append(overflow)
            card.focus_order.append(overflow.get_first_child())
            self.overflows[key] = overflow
            buttons = [upgrade, remove]
            if lifecycle == 'pending_removal':
                for title, action in (('Finalize removal', 'finalize'), ('Cancel removal', 'cancel_removal')):
                    operation = Gtk.Button(label=_(title))
                    if action == 'cancel_removal':
                        operation.set_tooltip_text(_('Makes the resource available again. Repaired Theme references are not restored.'))
                        actions.append(self._label('Repaired Theme references are not restored.'))
                    operation.connect('clicked', lambda _button, value=action: self.callbacks['library_lifecycle'](value, self.category, key))
                    actions.append(operation)
                    buttons.append(operation)
                remove.set_visible(False)
            self.mutation_buttons[key] = buttons
            for button in buttons:
                button.set_sensitive(getattr(button, 'lifecycle_allowed', True) and self.mutations.resource_allowed)
        self.cards[key] = card
        return card

    def dependency_details(self, key):
        self.grid.set_model(None)
        clear(self.content)
        self.content.append(text('Resource dependencies', 'section-title'))
        self.content.append(self._label(key))
        for user in self.snapshot.dependencies.get(self.category, {}).get(key, []):
            self.content.append(self._label(user['name']))
        for runtime in self.snapshot.runtime_dependencies:
            if runtime.uses(self.category, key):
                self.content.append(self._label(_('Running Theme') + ': ' + runtime.theme_id))
        back = Gtk.Button(label=_('Back to resources'))
        back.connect('clicked', lambda *_: self.callbacks['library_category'](self.category))
        self.content.append(back)
        back.grab_focus()

    def confirm_removal(self, key):
        if key in self.overflows:
            self.overflows[key].popdown()
        self.grid.set_model(None)
        clear(self.content)
        users = self.snapshot.dependencies.get(self.category, {}).get(key, [])
        self.content.append(text('Review removal', 'section-title'))
        self.content.append(self._label(key))
        self.content.append(self._label((_('%d Saved Theme') if len(users) == 1 else _('%d Saved Themes')) % len(users)))
        for user in users:
            self.content.append(self._label(user['name']))
        replacements = [item for item, record in getattr(self.snapshot, self.category).items() if item != key and record.get('lifecycle', 'active') == 'active']
        self.replacement_keys = replacements
        self.replacement = None
        active = [item for item in self.snapshot.runtime_dependencies if item.uses(self.category, key)]
        if active:
            self.content.append(self._label(_('Running Theme') + ': ' + ', '.join(item.theme_id for item in active)))
            self.content.append(self._label('Repair saved Themes first. Apply the replacement or Stop Theme before final removal.'))
            for title, action in (('Apply replacement', 'apply'), ('Stop Theme', 'stop')):
                button = Gtk.Button(label=_(title))
                button.connect('clicked', lambda _button, value=action: self.callbacks['library_runtime'](value, active[0].theme_id))
                button.set_sensitive(self.mutations.runtime_allowed)
                self.execution_buttons.append(button)
                self.content.append(button)
        if self.snapshot.runtime_error:
            self.content.append(self._label('Runtime connection could not be verified. Removal waits for reconnection.'))
        repair = {'expected_revision': self.snapshot.theme_revision}
        if active:
            repair['repair_only'] = True
        if users and self.category == 'plugins':
            repair['disable'] = True
            self.content.append(self._label('Disable this feature in every listed Theme before removal. Runtime changes wait for Apply.'))
        elif users:
            self.content.append(self._label('Choose a replacement for every listed Theme. Runtime changes wait for Apply.'))
            names = [getattr(self.snapshot, self.category)[item].get('display_name',
                     getattr(self.snapshot, self.category)[item].get('name', item)) for item in replacements]
            self.replacement = Gtk.DropDown.new_from_strings(names)
            self.replacement.set_hexpand(True)
            self.content.append(self.replacement)
        else:
            self.content.append(self._label('No Theme depends on this resource.'))
        cancel = Gtk.Button(label=_('Cancel'))
        cancel.connect('clicked', lambda *_: self.callbacks['library_category'](self.category))
        self.content.append(cancel)
        self.confirm_button = Gtk.Button(label=_('Repair saved Themes' if active else 'Disable and uninstall' if users and self.category == 'plugins' else
                                                 'Replace and uninstall' if users else 'Uninstall'))
        self.confirm_button.lifecycle_allowed = not users or self.category == 'plugins' or bool(replacements)
        self.confirm_button.set_sensitive(self.confirm_button.lifecycle_allowed and self.mutations.resource_allowed)
        def confirmed(*_args):
            if self.disposed:
                return
            if self.replacement is not None:
                repair['replacement'] = replacements[self.replacement.get_selected()]
            self.callbacks['library_remove'](self.category, key, dict(repair))
        self.confirm_button.connect('clicked', confirmed)
        self.content.append(self.confirm_button)
        cancel.grab_focus()

    def update_mutations(self, value):
        self.mutations = value
        for buttons in self.mutation_buttons.values():
            for button in buttons:
                button.set_sensitive(getattr(button, 'lifecycle_allowed', True) and value.resource_allowed)
        if self.confirm_button:
            self.confirm_button.set_sensitive(self.confirm_button.lifecycle_allowed and value.resource_allowed)
        for button in self.execution_buttons:
            button.set_sensitive(value.runtime_allowed)

    def update_job(self, job):
        self.progress.set_visible(bool(job))
        phases = {'queued': 'Queued', 'validating': 'Validating package…', 'committing': 'Committing resource transaction…',
                  'refreshing': 'Refreshing resources…', 'completed': 'Completed', 'failed': 'Failed'}
        self.progress_label.set_text(_(phases[job.phase]) if job else '')

    def responsive(self, compact):
        self.compact = compact
        self.toolbar.set_orientation(Gtk.Orientation.VERTICAL if compact else Gtk.Orientation.HORIZONTAL)
        self.set_width(420 if compact else 720)

    def set_width(self, width):
        thresholds = self.layout['library']
        self.toolbar.set_orientation(Gtk.Orientation.VERTICAL if width < thresholds['threeColumnWidth'] else Gtk.Orientation.HORIZONTAL)
        columns = 1 if width < thresholds['twoColumnWidth'] else 2 if width < thresholds['threeColumnWidth'] else 3
        self.grid.set_min_columns(columns)
        self.grid.set_max_columns(columns)
        self.schedule_visible()

    def detach(self):
        self.disposed = True
        if self.visible_tick is not None:
            self.grid.remove_tick_callback(self.visible_tick)
            self.visible_tick = None
        self.grid.set_model(None)

    def dispose(self):
        self.detach()
