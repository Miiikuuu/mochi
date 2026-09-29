"""Native Manager shell page."""
from pathlib import Path
from gi.repository import Gdk, GdkPixbuf, GLib, Gtk, Pango
from manager_i18n import _
from manager_widgets import NAMES, box, combo, css, field, icon, icon_button, label, portrait

from mochi_paths import DATA as ROOT
CATEGORIES = {'characters': 'Characters', 'assets': 'Assets', 'plugins': 'Plugins'}


class ShellPage:
    def character_name(self, key):
        return NAMES.get(key, self.state.catalog[key]['name'])


    def theme_name(self, theme):
        return 'Lime Squad' if theme['id'] == 'lime-squad' and theme['name'] == '酸橙小队' else theme['name']


    def bubble_name(self, key):
        return {'default': 'Default', 'lime-notes': 'Lime Notes'}.get(key, self.state.bubbles.get(key, key))


    def plugin_name(self, key):
        return {'translation': 'Translation'}.get(key, self.state.plugins.get(key, {}).get('name', key))


    def action(self, name, symbol, hint, callback, text=None):
        button = self.register(name, icon_button(symbol, hint, text))
        button.connect('clicked', callback)
        return button


    def build(self):
        header = Gtk.HeaderBar(title='Mochi', show_close_button=True)
        brand = css(box(), 'brand')
        brand.set_size_request(62, -1)
        mark = Gtk.Image.new_from_pixbuf(GdkPixbuf.Pixbuf.new_from_file_at_scale(
            str(ROOT/'assets/brand/mochi-icon.svg'), 42, 42, True))
        mark.set_halign(Gtk.Align.CENTER)
        mark.set_valign(Gtk.Align.CENTER)
        brand.pack_start(mark, True, True, 0)
        header.pack_start(brand)
        self.crumbs = box(spacing=8)
        self.crumbs.set_hexpand(True)
        header.set_custom_title(self.crumbs)
        self.window.set_titlebar(header)
        root = box(spacing=0)
        self.window.add(root)
        self.sidebar = css(box(True, 10), 'sidebar')
        self.sidebar.set_size_request(76, -1)
        root.pack_start(self.sidebar, False, False, 0)
        self.nav = {}
        for name, symbol, title in [('themes', 'layers', 'Themes'), ('library', 'library', 'Library'), ('settings', 'settings', 'Settings')]:
            button = self.action('nav:'+name, symbol, title, lambda _, area=name: self.navigate(area))
            css(button, 'nav-button')
            button.set_halign(Gtk.Align.CENTER)
            self.nav[name] = button
            (self.sidebar.pack_end if name == 'settings' else self.sidebar.pack_start)(button, False, False, 0)
        self.body = box(True, 0)
        root.pack_start(self.body, True, True, 0)
        self.config_banner = css(box(True, 4), 'config-banner')
        self.config_banner.set_no_show_all(True)
        self.config_banner_label = label('', 'config-banner-text')
        self.config_banner_label.set_line_wrap(True)
        self.config_banner.pack_start(self.config_banner_label, False, False, 0)
        self.config_banner_actions = box(spacing=4)
        self.config_banner.pack_start(self.config_banner_actions, False, False, 0)
        self.body.pack_start(self.config_banner, False, False, 0)
        self.scroll = Gtk.ScrolledWindow()
        # The viewport must be allowed to shrink before a four-column grid can
        # reflow to two columns; NEVER would propagate the grid's old minimum.
        self.scroll.set_policy(Gtk.PolicyType.EXTERNAL, Gtk.PolicyType.AUTOMATIC)
        self.scroll.set_hexpand(True)
        self.scroll.set_vexpand(True)
        self.content = css(box(True, 0), 'content')
        self.content.set_valign(Gtk.Align.START)
        self.scroll.add(self.content)
        self.body.pack_start(self.scroll, True, True, 0)
        self.error = css(Gtk.Label(xalign=0), 'error')
        self.error.set_line_wrap(True)
        self.error.set_max_width_chars(65)
        self.error.set_no_show_all(True)
        self.body.pack_start(self.error, False, False, 0)
        self.error_details = Gtk.Expander(label=_('Details'))
        self.error_details.set_no_show_all(True)
        detail_scroll = Gtk.ScrolledWindow()
        detail_scroll.set_size_request(-1, 110)
        self.error_detail_text = Gtk.TextView()
        self.error_detail_text.set_editable(False)
        self.error_detail_text.set_cursor_visible(False)
        self.error_detail_text.set_wrap_mode(Gtk.WrapMode.WORD_CHAR)
        detail_scroll.add(self.error_detail_text)
        self.error_details.add(detail_scroll)
        self.body.pack_start(self.error_details, False, False, 0)
        self.error.connect('hide', lambda *_: self.error_details.hide())
        self.footer = css(box(spacing=12), 'footer')
        self.footer.set_no_show_all(True)
        self.body.pack_end(self.footer, False, False, 0)
        self.status_group = box(spacing=12)
        state = box(spacing=8)
        self.state_icon = icon('circle-stop', 16)
        self.status = label('Stopped', 'muted', True)
        self.status.set_max_width_chars(30)
        self.spinner = Gtk.Spinner()
        self.spinner.set_no_show_all(True)
        state.pack_start(self.spinner, False, False, 0)
        state.pack_start(self.state_icon, False, False, 0)
        state.pack_start(self.status, True, True, 0)
        self.status_group.pack_start(state, True, True, 0)
        self.pending_label = label('Changes pending', 'pending')
        self.pending_label.set_no_show_all(True)
        self.status_group.pack_start(self.pending_label, False, False, 0)
        self.footer.pack_start(self.status_group, True, True, 0)
        self.stop = self.action('stop', 'stop', 'Stop theme and cancel its tasks', lambda *_: self.stop_theme())
        self.stop.set_no_show_all(True)
        self.footer.pack_end(self.stop, False, False, 0)
        self.start = css(Gtk.Button(), 'primary-action')
        self.register('start', self.start)
        start_box = box(spacing=8)
        self.start_icon = icon('play')
        self.start_label = label('Apply')
        self.start_label.set_no_show_all(True)
        start_box.pack_start(self.start_icon, False, False, 0)
        start_box.pack_start(self.start_label, False, False, 0)
        self.start.add(start_box)
        self.start.set_no_show_all(True)
        self.start.connect('clicked', self.apply)
        self.footer.pack_end(self.start, False, False, 0)
        self.status_group.show_all()
        self.stop.get_child().show_all()
        self.start.get_child().show_all()
        self.window.connect('size-allocate', self.resize)


    def heading(self, title, back=False):
        row = css(box(spacing=12), 'page-heading')
        if back:
            row.pack_start(self.action('back', 'back', 'Back', self.back), False, False, 0)
        text = label(title, 'heading', True)
        text.set_max_width_chars(36)
        text.set_tooltip_text(_(title))
        row.pack_start(text, True, True, 0)
        self.heading_actions = box(spacing=8)
        row.pack_end(self.heading_actions, False, False, 0)
        self.content.pack_start(row, False, False, 0)


    def crumb(self, title, callback=None, symbol=None):
        if self.crumbs.get_children():
            self.crumbs.pack_start(css(icon('chevron', 12), 'muted'), False, False, 0)
        if callback:
            button = css(Gtk.Button(), 'crumb')
            text = icon(symbol, 16) if symbol else label(title, 'muted')
            if not symbol:
                text.set_max_width_chars(18)
            button.add(text)
            button.get_accessible().set_name(title)
            button.set_tooltip_text(_(title))
            button.connect('clicked', callback)
            widget = button
        else:
            widget = icon(symbol, 16) if symbol else label(title)
            if not symbol:
                widget.set_max_width_chars(18)
            widget.set_tooltip_text(_(title))
        self.crumbs.pack_start(widget, False, False, 0)


    def breadcrumbs(self):
        for child in self.crumbs.get_children():
            child.destroy()
        area = self.area
        self.crumb(area.title(), lambda *_: self.navigate(area), {'themes': 'layers', 'library': 'library', 'settings': 'settings'}[area])
        if area == 'themes' and self.screen not in ('list', 'new'):
            self.crumb(self.theme_name(self.theme), lambda *_: self.open_theme(self.theme['id']))
        if self.detail:
            tab = self.library_tab if area == 'library' else self.tab
            title = CATEGORIES[tab]
            self.crumb(title, self.back)
            self.crumb(self.character_name(self.detail) if tab == 'characters' else self.bubble_name(self.detail) if tab == 'assets' else self.plugin_name(self.detail))
        elif self.screen in ('options', 'new', 'add', 'replace'):
            self.crumb({'options': 'Theme settings', 'new': 'New theme', 'add': 'Add character', 'replace': 'Change bubble'}[self.screen])
        self.crumbs.show_all()


    def grid(self, cards, kind='characters'):
        grid = Gtk.Grid(column_spacing=16, row_spacing=20, column_homogeneous=True)
        grid.set_hexpand(True)
        self.grids.append((grid, cards, kind))
        self.content.pack_start(grid, False, False, 0)


    def render(self):
        self.stop_action_preview()
        self.controller.set_translation_editing(
            self.theme['id'] if self.area == 'themes' and self.tab == 'plugins' and self.detail == 'translation' else None)
        self.guard = True
        for child in self.content.get_children():
            child.destroy()
        self.new_page()
        self.grids = []
        self.detail_box = self.detail_art = self.category_bar = None
        self.breadcrumbs()
        for key, button in self.nav.items():
            context = button.get_style_context()
            (context.add_class if key == self.area else context.remove_class)('active')
            button.get_accessible().set_description('Current page' if key == self.area else '')
        if self.area == 'settings':
            self.render_settings()
        elif self.detail:
            self.render_detail()
        elif self.area == 'library':
            self.render_library()
        else:
            self.render_themes()
        self.content.show_all()
        self.reflow()
        self.guard = False
        self.update_actions()


    def resize(self, _window, allocation):
        band = 0 if allocation.width <= 580 else 1 if allocation.width <= 760 else 2
        if band != self.width_band:
            self.width_band = band
            # A resize requested from inside size-allocate can be discarded by
            # GTK. Reflow after the allocation so the next layout sees it.
            GLib.idle_add(self.reflow)


    def reflow(self):
        if self.closed:
            return False
        band = self.width_band
        padding = [16, 24, 32][band]
        self.sidebar.set_size_request(56 if band == 0 else 76, -1)
        for widget in (self.content, self.error, self.error_details, self.footer):
            widget.set_margin_start(padding)
            widget.set_margin_end(padding)
        crumbs = self.crumbs.get_children()
        for index, crumb in enumerate(crumbs):
            crumb.set_visible(band != 0 or index == 0 or index >= len(crumbs)-2)
        if self.category_bar:
            self.category_bar.set_orientation(Gtk.Orientation.VERTICAL if band == 0 else Gtk.Orientation.HORIZONTAL)
        if self.detail_box:
            self.detail_box.set_orientation(Gtk.Orientation.VERTICAL if band == 0 else Gtk.Orientation.HORIZONTAL)
            self.detail_art.set_size_request(-1 if band == 0 else 180 if band == 1 else 240, 240 if band < 2 else 280)
        self.status_group.set_orientation(Gtk.Orientation.VERTICAL if band == 0 else Gtk.Orientation.HORIZONTAL)
        self.status_group.set_spacing(6 if band == 0 else 12)
        for grid, cards, kind in self.grids:
            columns = (4 if band == 2 else 2) if kind == 'characters' else (2 if band == 2 or (kind == 'bubbles' and band == 1) else 1)
            for child in grid.get_children():
                grid.remove(child)
            for i, card in enumerate(cards):
                grid.attach(card, i % columns, i // columns, 1, 1)
            if cards and len(cards) < columns:
                grid.attach(Gtk.Box(), columns-1, 0, 1, 1)
            grid.show_all()
        return False
