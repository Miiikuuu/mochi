"""Native Manager themes page."""
from pathlib import Path
from gi.repository import Gdk, GdkPixbuf, GLib, Gtk, Pango
from manager_widgets import NAMES, box, combo, css, field, icon, icon_button, label, portrait

from mochi_paths import DATA as ROOT
CATEGORIES = {'characters': 'Characters', 'assets': 'Assets', 'plugins': 'Plugins'}


class ThemesPage:
    def render_themes(self):
        if self.screen == 'list':
            self.heading('Themes')
            self.heading_actions.pack_start(self.action('theme_import', 'import', 'Import theme configuration', lambda *_: self.import_file('theme')), False, False, 0)
            self.heading_actions.pack_start(self.action('new', 'plus', 'New theme', lambda *_: self.show_screen('new')), False, False, 0)
            self.theme_cards()
        elif self.screen == 'new':
            self.heading('New theme', True)
            self.content.pack_start(label('Name'), False, False, 0)
            name = self.register('new_name', Gtk.Entry())
            name.set_max_length(60)
            name.set_margin_top(8)
            name.get_accessible().set_name('Name')
            name.connect('activate', self.create)
            self.content.pack_start(name, False, False, 0)
            actions = box(spacing=8)
            actions.set_halign(Gtk.Align.END)
            actions.set_margin_top(24)
            actions.pack_start(self.action('cancel', 'close', 'Cancel', lambda *_: self.navigate('themes')), False, False, 0)
            actions.pack_start(css(self.action('create', 'check', 'Create theme', self.create), 'primary-action'), False, False, 0)
            self.content.pack_start(actions, False, False, 0)
        elif self.screen == 'options':
            self.settings_page(theme=True)
        elif self.screen == 'add':
            self.heading('Add character', True)
            self.heading_actions.pack_start(self.action('import', 'import', 'Import character package', lambda *_: self.import_file('character')), False, False, 0)
            self.grid([self.character_card(key, add=True) for key in self.state.catalog if key not in self.theme['characters']])
        elif self.screen == 'replace':
            self.heading('Change bubble', True)
            self.grid([self.bubble_card(key, key == self.theme['bubble']) for key in self.state.bubbles], 'bubbles')
        else:
            self.heading(self.theme_name(self.theme))
            self.heading_actions.pack_start(self.action('settings', 'sliders', 'Theme settings', lambda *_: self.show_screen('options')), False, False, 0)
            self.categories()
            if self.tab == 'characters':
                self.grid([self.character_card(key) for key in self.theme['characters']])
            elif self.tab == 'assets':
                self.grid([self.bubble_card(self.theme['bubble'], True)], 'bubbles')
            else:
                self.render_connections()

    def theme_cards(self):
        cards = []
        for theme in self.state.themes:
            button = css(Gtk.Button(), 'theme-card')
            self.register('theme:'+theme['id'], button)
            button.get_accessible().set_name('Open '+self.theme_name(theme))
            card = box(True, 0)
            art = css(box(spacing=4), 'theme-art')
            for key in theme['characters']:
                picture = portrait(key, self.state.catalog[key], 90, 66)
                picture.set_valign(Gtk.Align.END)
                art.pack_start(picture, True, True, 0)
            card.pack_start(art, False, False, 0)
            caption = css(box(spacing=12), 'theme-caption')
            copy = box(True, 8)
            name = label(self.theme_name(theme), expand=True)
            name.set_max_width_chars(1)
            copy.pack_start(name, False, False, 0)
            metrics = css(box(spacing=8), 'muted')
            for symbol, value in [('users', len(theme['characters'])), ('plug', len(theme['plugins']))]:
                metrics.pack_start(icon(symbol, 14), False, False, 0)
                metrics.pack_start(label(str(value)), False, False, 0)
            if self.live and self.live['theme']['id'] == theme['id']:
                metrics.pack_start(css(icon('circle-play', 14), 'live'), False, False, 0)
                metrics.pack_start(label('Running'), False, False, 0)
            copy.pack_start(metrics, False, False, 0)
            caption.pack_start(copy, True, True, 0)
            caption.pack_end(icon('chevron', 16), False, False, 0)
            card.pack_start(caption, False, False, 0)
            button.add(card)
            button.connect('clicked', lambda _, key=theme['id']: self.open_theme(key))
            cards.append(button)
        self.grid(cards, 'themes')
