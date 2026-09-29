"""Native Manager library page."""
from pathlib import Path
from gi.repository import Gdk, GdkPixbuf, GLib, Gtk, Pango
from manager_i18n import _
from manager_widgets import NAMES, box, combo, css, field, icon, icon_button, label, portrait

from mochi_paths import DATA as ROOT
CATEGORIES = {'characters': 'Characters', 'assets': 'Assets', 'plugins': 'Plugins'}


class LibraryPage:
    def render_library(self):
        self.heading('Library')
        self.categories(library=True)
        if self.library_tab == 'characters':
            self.grid([self.character_card(key, library=True) for key in self.state.catalog])
        elif self.library_tab == 'assets':
            kind = label('Bubbles', 'category-name')
            kind.set_margin_bottom(16)
            self.content.pack_start(kind, False, False, 0)
            self.grid([self.bubble_card(key) for key in self.state.bubbles], 'bubbles')
        else:
            self.render_connections(library=True)

    def categories(self, library=False):
        current = self.library_tab if library else self.tab
        items = [('characters', 'Characters', 'users', len(self.state.catalog) if library else str(len(self.theme['characters']))+' / 4'),
                 ('assets', 'Assets', 'image', len(self.state.bubbles) if library else 1),
                 ('plugins', 'Plugins', 'plug', len(self.plugin_keys()))]
        bar = css(box(spacing=16), 'category-bar')
        self.category_bar = bar
        name, count = next((title, count) for key, title, _, count in items if key == current)
        title = box(spacing=10)
        title.pack_start(label(name, 'category-name'), False, False, 0)
        title.pack_start(label(str(count), 'muted'), False, False, 0)
        bar.pack_start(title, True, True, 0)
        actions = box(spacing=12)
        tabs = css(box(spacing=3), 'tabs')
        self.tab_buttons = []
        for key, title, symbol, _ in items:
            button = self.action('tab:'+key, symbol, title, lambda _, key=key: self.change_tab(key))
            css(button, 'tab-button')
            if key == current:
                css(button, 'selected')
            button.get_accessible().set_description('Selected category' if key == current else 'Category')
            button.connect('key-press-event', self.tab_key)
            tabs.pack_start(button, False, False, 0)
            self.tab_buttons.append(button)
        actions.pack_start(tabs, False, False, 0)
        if library:
            kind = {'characters': 'character', 'assets': 'bubble', 'plugins': 'plugin'}[current]
            button = self.action('import', 'import', 'Import '+kind+' package', lambda *_: self.import_file(kind))
            actions.pack_end(button, False, False, 0)
        elif not library and current == 'characters':
            full = len(self.theme['characters']) >= 4
            button = self.action('add', 'plus', '4 of 4 characters. Disable one before adding another.' if full else 'Add character', self.add_characters)
            if full:
                css(button, 'unavailable')
            actions.pack_end(button, False, False, 0)
        elif not library and current == 'assets':
            actions.pack_end(self.action('replace', 'replace', 'Change bubble', lambda *_: self.show_screen('replace')), False, False, 0)
        bar.pack_end(actions, False, False, 0)
        self.content.pack_start(bar, False, False, 0)


    def character_card(self, key, add=False, library=False):
        name = self.character_name(key)
        button = css(Gtk.Button(), 'character-card')
        button.set_tooltip_text(name)
        primary = not library and not add and key == self.theme['primary']
        button.get_accessible().set_name(('Add ' if add else 'Open ')+name+(', primary character' if primary else ''))
        self.register('card:'+key, button)
        card = box(True, 0)
        art = Gtk.Overlay()
        art_box = css(box(True, 0), 'character-art')
        art_box.set_size_request(-1, 174)
        picture = portrait(key, self.state.catalog[key])
        picture.set_margin_bottom(24)
        picture.set_valign(Gtk.Align.END)
        art_box.pack_end(picture, False, False, 0)
        art.add(art_box)
        if primary:
            crown = css(icon('crown', 16), 'muted')
            crown.set_halign(Gtk.Align.START)
            crown.set_valign(Gtk.Align.START)
            crown.set_margin_start(12)
            crown.set_margin_top(12)
            crown.set_tooltip_text(_('Primary character'))
            art.add_overlay(crown)
        card.pack_start(art, False, False, 0)
        caption = label(name, 'caption', True)
        caption.set_max_width_chars(1)
        card.pack_start(caption, False, False, 0)
        button.add(card)
        button.connect('clicked', lambda *_: self.add_character(key) if add else self.open_detail(key))
        return button


    def bubble_art(self, key, detail=False):
        art = css(box(True, 0), 'bubble-art')
        art.set_size_request(-1, 224 if detail else 174)
        bubble = css(box(True, 6), 'bubble-sample')
        if key == 'lime-notes':
            css(bubble, 'lime-sample')
            flower = css(icon('flower', 18), 'flower')
            flower.set_halign(Gtk.Align.START)
            bubble.pack_start(flower, False, False, 0)
        text = Gtk.Label(label='A little company\nfor your day.')
        text.set_line_wrap(True)
        bubble.pack_start(text, False, False, 0)
        bubble.set_halign(Gtk.Align.CENTER)
        bubble.set_valign(Gtk.Align.CENTER)
        art.pack_start(bubble, True, False, 20)
        return art


    def bubble_card(self, key, selected=False):
        button = css(Gtk.Button(), 'character-card')
        button.get_accessible().set_name('Open '+self.bubble_name(key)+(', selected' if selected else ''))
        card = box(True, 0)
        card.pack_start(self.bubble_art(key), False, False, 0)
        caption = css(box(spacing=8), 'caption')
        caption.pack_start(label(self.bubble_name(key), expand=True), True, True, 0)
        if selected:
            caption.pack_end(icon('check', 16), False, False, 0)
        card.pack_start(caption, False, False, 0)
        button.add(card)
        self.register('bubble:'+key, button)
        button.connect('clicked', lambda *_: self.open_detail(key))
        return button


    def usage(self, predicate, tab=None):
        fields = box(True, 12)
        fields.pack_start(label('Used in', 'category-name'), False, False, 0)
        used = [theme for theme in self.state.themes if predicate(theme)]
        for theme in used:
            button = Gtk.Button(label=self.theme_name(theme))
            button.get_child().set_ellipsize(Pango.EllipsizeMode.END)
            button.get_child().set_max_width_chars(25)
            button.connect('clicked', lambda _, key=theme['id']: self.open_theme(key, tab))
            self.register('used:'+theme['id'], button)
            fields.pack_start(button, False, False, 0)
        if not used:
            fields.pack_start(label('No themes yet', 'muted'), False, False, 0)
        return fields
