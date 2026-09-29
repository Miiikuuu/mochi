"""Cast page: owns only its presentation controls."""
from manager_v2_common import Gdk, GLib, GObject, Gtk, Page, styled, text
from manager_v2_common import thumbnail_pixbuf
from manager_i18n import _


class CastPage(Page):
    def __init__(self, *args):
        super().__init__(*args)
        self.cast_rows = {}
        self.cast_cards = {}
        self.add_character_choices = {}
        self._cast(self.state.draft.value)

    def _cast(self, theme):
        self.workspace_content.append(text('Cast', 'section-title'))
        self.workspace_content.append(text('Characters in this Theme', 'section-hint'))
        for index, key in enumerate(theme['characters']):
            model = self.character_models.get(key)
            card = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['sm']), 'cast-card')
            card.set_margin_top(self.layout['spacing']['card'])
            heading = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['card'])
            thumbnail = styled(Gtk.Picture(), 'cast-thumbnail')
            thumbnail.set_content_fit(Gtk.ContentFit.CONTAIN)
            thumbnail.set_size_request(46, 56)
            if model and model.thumbnail:
                try:
                    thumbnail.set_paintable(Gdk.Texture.new_for_pixbuf(thumbnail_pixbuf(model.thumbnail)))
                except GLib.Error:
                    pass
            heading.append(thumbnail)
            content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['xs'])
            content.set_hexpand(True)
            name = text(model.display_name if model else key, 'theme-row-name')
            name.set_ellipsize(3)
            content.append(name)
            technical_id = text(key, 'muted')
            technical_id.set_ellipsize(3)
            technical_id.set_tooltip_text(key)
            content.append(technical_id)
            heading.append(content)
            if key == theme['primary']:
                heading.append(text('PRIMARY', 'cast-role'))
            card.append(heading)
            if not model or model.lifecycle != 'active':
                card.append(text('Missing character. Add a replacement, then disable this character.' if not model or model.lifecycle == 'missing' else 'Pending removal. Replace or disable this character before Apply.', 'section-hint'))
            controls = styled(Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['sm']), 'cast-controls')
            buttons = {}
            for action, symbol, label, enabled in (
                    ('primary', 'starred-symbolic', 'Set as primary', key != theme['primary']),
                    ('up', 'go-up-symbolic', 'Move up', index > 0),
                    ('down', 'go-down-symbolic', 'Move down', index+1 < len(theme['characters'])),
                    ('disable', 'list-remove-symbolic', 'Disable character', len(theme['characters']) > 1)):
                button = Gtk.Button(icon_name=symbol)
                button.set_tooltip_text(_(label))
                button.update_property([Gtk.AccessibleProperty.LABEL], [_(label)])
                button.set_sensitive(enabled)
                button.connect('clicked', lambda _button, operation=action, character=key:
                               self.callbacks['cast'](operation, character))
                controls.append(button)
                buttons[action] = button
            card.append(controls)
            if not model or model.lifecycle == 'missing':
                repair = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['sm'])
                replace = Gtk.MenuButton(label=_('Replace'))
                options = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
                for replacement, candidate in self.character_models.items():
                    if candidate.lifecycle == 'active' and replacement not in theme['characters']:
                        button = Gtk.Button(label=candidate.display_name)
                        button.connect('clicked', lambda _b, old=key, new=replacement: self.callbacks['cast']('replace', (old, new)))
                        options.append(button)
                popover = Gtk.Popover(); popover.set_child(options); replace.set_popover(popover)
                repair.append(replace)
                locate = Gtk.Button(label=_('Locate'))
                locate.connect('clicked', lambda _b, old=key: self.callbacks['cast']('locate', old))
                repair.append(locate)
                card.append(repair)
            drag = Gtk.DragSource.new()
            drag.set_actions(Gdk.DragAction.MOVE)
            drag.connect('prepare', lambda _source, _x, _y, identity=key:
                         Gdk.ContentProvider.new_for_value(identity))
            card.add_controller(drag)
            drop = Gtk.DropTarget.new(GObject.TYPE_STRING, Gdk.DragAction.MOVE)
            drop.connect('drop', lambda _target, value, _x, _y, identity=key:
                         self._drop_character(value, identity))
            card.add_controller(drop)
            self.workspace_content.append(card)
            self.cast_rows[key] = buttons
            self.cast_cards[key] = card
        self.add_character_button = Gtk.MenuButton(label=_('Add character'))
        self.add_character_button.set_focusable(True)
        self.add_character_button.add_css_class('cast-add')
        available = [(key, item) for key, item in self.character_models.items() if key not in theme['characters'] and item.lifecycle == 'active']
        self.add_character_button.set_sensitive(len(theme['characters']) < 4 and bool(available))
        choices = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['sm'])
        choices.add_css_class('cast-choices')
        for key, item in available:
            button = Gtk.Button(label=item.display_name)
            button.connect('clicked', lambda _button, character=key: self.callbacks['cast']('add', character))
            choices.append(button)
            self.add_character_choices[key] = button
        popover = Gtk.Popover()
        popover.set_child(choices)
        self.add_character_button.set_popover(popover)
        self.workspace_content.append(self.add_character_button)
        self.workspace_content.append(text(_('Up to four characters per Theme.'), 'field-help'))


    def _drop_character(self, source, target):
        if source == target or source not in self.state.draft.value['characters']:
            return False
        self.callbacks['cast']('reorder', (source, target))
        return True
