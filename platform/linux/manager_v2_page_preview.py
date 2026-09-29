"""Preview page: owns only its presentation controls."""
from manager_v2_common import GdkPixbuf, GLib, Gtk, Page, styled, text, preview_geometry, thumbnail_pixbuf
from manager_i18n import _


class PreviewPage(Page):
    def __init__(self, *args):
        super().__init__(*args)
        self.preview_pictures = []
        self.preview_tiles = {}
        self.preview_primary_badges = {}
        self.active_tab = 'preview'
        self._preview(self.state.draft.value)

    def _preview(self, theme):
        self.workspace_content.append(text('Preview Stage', 'section-title'))
        hint = text('A still preview of this Theme. The desktop pet is not started here.', 'section-hint')
        hint.set_wrap(True)
        self.workspace_content.append(hint)
        stage = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['md']), 'preview-stage')
        stage.set_margin_top(self.layout['spacing']['stageTop'])
        stage.append(text('STATIC PREVIEW', 'stage-caption'))
        self.preview_size_label = text(_('Character size: %d px') % theme['size'], 'stage-size')
        stage.append(self.preview_size_label)
        self.preview_size_scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 128, 512, 1)
        self.preview_size_scale.set_draw_value(False)
        self.preview_size_scale.set_value(theme['size'])
        self.preview_size_scale.update_property([Gtk.AccessibleProperty.LABEL], [_('Character size')])
        self.preview_size_scale.connect('value-changed', lambda scale:
                                        self.callbacks['behavior']('size', round(scale.get_value())))
        stage.append(self.preview_size_scale)
        gallery = styled(Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['gallery']), 'preview-art')
        self.preview_art = gallery
        gallery.set_homogeneous(True)
        gallery.set_hexpand(True)
        self.preview_pictures = []
        for key in theme['characters']:
            model = self.character_models.get(key)
            tile = styled(Gtk.Button(), 'preview-character')
            tile.set_hexpand(True)
            tile.connect('clicked', lambda _button, character=key: self.callbacks['select_character'](character))
            tile.update_property([Gtk.AccessibleProperty.LABEL], [model.display_name if model else key])
            inside = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['tiny'])
            tile.set_child(inside)
            art_slot = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL), 'preview-art-slot')
            art_slot.set_valign(Gtk.Align.END)
            art_slot.set_size_request(-1, preview_geometry(theme['size'], self.compact, self.layout).baseline)
            spacer = Gtk.Box()
            spacer.set_vexpand(True)
            art_slot.append(spacer)
            picture = styled(Gtk.Image(), 'preview-portrait')
            picture.character_id = key
            picture.set_halign(Gtk.Align.CENTER)
            picture.set_valign(Gtk.Align.END)
            data = model.thumbnail if model else None
            if data:
                try:
                    picture.source_pixbuf = thumbnail_pixbuf(data)
                except GLib.Error:
                    pass
            art_slot.append(picture)
            inside.append(art_slot)
            caption = text(model.display_name if model else key, 'preview-name')
            caption.set_ellipsize(3)
            caption.set_xalign(.5)
            inside.append(caption)
            if key == theme['primary']:
                badge = text('PRIMARY', 'preview-primary')
                badge.set_xalign(.5)
                inside.append(badge)
                self.preview_primary_badges[key] = badge
            gallery.append(tile)
            self.preview_pictures.append(picture)
            self.preview_tiles[key] = tile
        self.update_preview_size(theme['size'])
        self.select_preview_character(self.state.route.detail)
        stage.append(gallery)
        self.preview_stage = stage
        self.workspace_content.append(stage)
        field = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL), 'field-group')
        field.append(text('Theme name', 'field-label'))
        self.name_entry = Gtk.Entry(text=theme['name'])
        self.name_entry.set_hexpand(True)
        self.name_entry.connect('changed', lambda entry: self.callbacks['edit_name'](entry.get_text()))
        field.append(self.name_entry)
        field.append(text('Saved changes stay in the draft until you apply them.', 'field-help'))
        self.workspace_content.append(field)


    def update_preview_size(self, size):
        if not self.preview_pictures or self.active_tab != 'preview':
            return
        geometry = preview_geometry(size, self.compact, self.layout)
        width, height = geometry.width, geometry.height
        for picture in self.preview_pictures:
            source = getattr(picture, 'source_pixbuf', None)
            if source:
                ratio = min(width / source.get_width(), height / source.get_height())
                scaled = source.scale_simple(max(1, round(source.get_width() * ratio)),
                                             max(1, round(source.get_height() * ratio)),
                                             GdkPixbuf.InterpType.BILINEAR)
                picture.set_from_pixbuf(scaled)
                picture.set_pixel_size(width)
            else:
                picture.set_size_request(width, height)
        self.preview_size_label.set_text(_('Character size: %d px') % size)


    def select_preview_character(self, key):
        for character, tile in self.preview_tiles.items():
            if character == key:
                tile.add_css_class('selected')
            else:
                tile.remove_css_class('selected')


    def responsive(self, compact):
        super().responsive(compact)
        for picture in self.preview_pictures:
            picture.get_parent().set_size_request(-1, preview_geometry(self.state.draft.value['size'], compact, self.layout).baseline)
        self.update_preview_size(self.state.draft.value['size'])
