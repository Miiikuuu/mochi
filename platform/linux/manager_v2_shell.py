"""GTK4 navigation shell. Concrete controls belong to pages/components."""
from manager_v2_common import Gdk, GdkPixbuf, GLib, Gtk, Pango, styled, text
from manager_i18n import _, error_parts
from manager_v2_page_preview import PreviewPage
from manager_v2_page_cast import CastPage
from manager_v2_page_behavior import BehaviorPage
from manager_v2_page_bubble import BubblePage
from manager_v2_page_features import FeaturesPage
from manager_v2_page_library import LibraryPage
from manager_v2_page_connections import ConnectionsPage
from manager_v2_page_settings import SettingsPage
from manager_v2_banner import Banner
from manager_v2_repair_banner import RepairBanner
from manager_v2_undo_toast import Undo
from manager_v2_command_bar import CommandBar

PAGES = {'preview': PreviewPage, 'cast': CastPage, 'behavior': BehaviorPage,
         'bubble': BubblePage, 'features': FeaturesPage}


class Shell(Gtk.ApplicationWindow):
    def __init__(self, app, state, callbacks, css, layout, brand_icon=None):
        super().__init__(application=app, title='Mochi Manager')
        self.state = state
        self.callbacks = callbacks
        self.layout = layout
        gap = layout['spacing']
        self.set_default_size(layout['defaultWidth'], layout['defaultHeight'])
        self.set_size_request(layout['minimumWidth'], layout['minimumHeight'])
        self.add_css_class('mochi-v2')
        provider = Gtk.CssProvider()
        provider.load_from_data(css)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider,
                                                   Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.provider = provider
        self.material_css_class = f'bubble-preview-{id(self):x}'
        self.add_css_class(self.material_css_class)
        self.material_provider = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), self.material_provider,
                                                   Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1)
        self.rail_buttons = {}
        self.tabs = {}
        self.sidebar_rows = {}
        self.page = None
        self.character_models = {}
        self.resources = ((), None, None, {})
        self.library_snapshot = None
        self.library_job = None
        self.library_states = {}
        self.compact = None
        self.surface_width_handler = None
        self.active_area = 'themes'
        self.active_tab = 'preview'

        self.root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.set_child(self.root)
        body = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        body.set_vexpand(True)
        self.root.append(body)
        self.rail = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=gap['lg']), 'rail')
        self.rail.set_size_request(layout['railWidth'], -1)
        body.append(self.rail)
        brand = styled(Gtk.Picture(), 'brand-icon')
        brand.set_content_fit(Gtk.ContentFit.CONTAIN)
        brand.set_size_request(layout['brandSize'], layout['brandSize'])
        if brand_icon:
            try:
                loader = GdkPixbuf.PixbufLoader.new_with_type('svg')
                loader.write(brand_icon)
                loader.close()
                icon = loader.get_pixbuf().scale_simple(layout['brandSize'], layout['brandSize'], GdkPixbuf.InterpType.BILINEAR)
                brand.set_paintable(Gdk.Texture.new_for_pixbuf(icon))
            except GLib.Error:
                pass
        brand.set_tooltip_text('Mochi')
        self.rail.append(brand)
        for area, symbol, title in (
                ('themes', 'view-grid-symbolic', 'Themes'),
                ('library', 'folder-symbolic', 'Library'),
                ('connections', 'network-workgroup-symbolic', 'Connections'),
                ('settings', 'emblem-system-symbolic', 'Settings')):
            button = Gtk.Button(icon_name=symbol)
            button.set_tooltip_text(_(title))
            button.update_property([Gtk.AccessibleProperty.LABEL], [_(title)])
            button.connect('clicked', lambda _button, key=area: self.callbacks['navigate'](key))
            self.rail.append(button)
            self.rail_buttons[area] = button

        self.sidebar = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL), 'theme-sidebar')
        self.sidebar.set_size_request(layout['themeSidebarWidth'], -1)
        body.append(self.sidebar)
        self.sidebar.append(text('WORKSPACE', 'eyebrow'))
        self.sidebar.append(text('Your Themes', 'sidebar-title'))
        self.theme_list = Gtk.ListBox()
        self.theme_list.set_selection_mode(Gtk.SelectionMode.SINGLE)
        self.theme_list.connect('row-activated', self._choose_row)
        theme_scroll = Gtk.ScrolledWindow()
        theme_scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        theme_scroll.set_vexpand(True)
        theme_scroll.set_child(self.theme_list)
        self.sidebar.append(theme_scroll)
        self.sidebar_count = text('', 'theme-count')
        self.sidebar.append(self.sidebar_count)

        self.workspace = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL), 'workspace')
        self.workspace.set_hexpand(True)
        body.append(self.workspace)
        self.banner = Banner(layout, callbacks)
        self.workspace.append(self.banner.root)
        self.repair = RepairBanner(layout, callbacks)
        self.workspace.append(self.repair.root)
        self.undo = Undo(layout, callbacks)
        self.workspace.append(self.undo.root)
        self.scroll = styled(Gtk.ScrolledWindow(), 'workspace-scroll')
        self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.scroll.set_hexpand(True)
        self.scroll.set_vexpand(True)
        self.workspace.append(self.scroll)
        self.workspace_content = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL), 'workspace-content')
        self.workspace_content.set_hexpand(True)
        self.scroll.set_child(self.workspace_content)

        self.error = text('', 'error')
        self.error.set_wrap(True)
        self.error.set_visible(False)
        self.workspace.append(self.error)
        self.error_details = Gtk.Expander(label=_('Details'))
        self.error_details.set_focusable(True)
        self.error_details.set_visible(False)
        self.error_detail_text = Gtk.TextView()
        self.error_detail_text.set_editable(False)
        self.error_detail_text.set_cursor_visible(False)
        self.error_detail_text.set_wrap_mode(Gtk.WrapMode.WORD_CHAR)
        details_scroll = Gtk.ScrolledWindow()
        details_scroll.set_size_request(-1, 110)
        details_scroll.set_child(self.error_detail_text)
        self.error_details.set_child(details_scroll)
        self.workspace.append(self.error_details)
        self.commandbar = CommandBar(layout, callbacks)
        self.workspace.append(self.commandbar.root)
        self.compact_themes = Gtk.MenuButton(label=_('Themes'))
        self.compact_themes.add_css_class('compact-themes')
        self.compact_list = Gtk.ListBox()
        self.compact_list.connect('row-activated', self._choose_row)
        popover = Gtk.Popover()
        popover.set_child(self.compact_list)
        self.compact_themes.set_popover(popover)
        self.set_theme_list()
        self.render_page()
        self.connect('realize', self._watch_surface_width)


    def _choose_row(self, _list, row):
        self.compact_themes.popdown()
        self.callbacks['choose'](row.theme_id)


    @staticmethod
    def _clear(box):
        child = box.get_first_child()
        while child:
            next_child = child.get_next_sibling()
            box.remove(child)
            child = next_child


    def set_theme_list(self):
        self.sidebar_rows.clear()
        for listing in (self.theme_list, self.compact_list):
            self._clear(listing)
            for theme in self.state.themes:
                row = Gtk.ListBoxRow()
                row.theme_id = theme['id']
                row.set_activatable(True)
                row.set_focusable(True)
                content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['xs'])
                name = text(theme['name'], 'theme-row-name')
                name.set_ellipsize(Pango.EllipsizeMode.END)
                name.set_max_width_chars(24)
                name.set_tooltip_text(theme['name'])
                content.append(name)
                content.append(text(_('1 character') if len(theme['characters']) == 1 else _('%d characters') % len(theme['characters']), 'muted'))
                row.set_child(content)
                listing.append(row)
                if listing is self.theme_list:
                    self.sidebar_rows[theme['id']] = row
                    if theme['id'] == self.state.draft.value['id']:
                        listing.select_row(row)
        self.sidebar_count.set_text(_('1 Theme') if len(self.state.themes) == 1 else _('%d Themes') % len(self.state.themes))


    def navigate(self, area):
        self.active_area = area
        self.render_page()


    def select_tab(self, tab):
        self.active_tab = tab
        self.state.route.tab = tab
        self.render_page()


    def set_theme_resources(self, features, bubble, translation, choices):
        self.resources = (features, bubble, translation, choices)
        if bubble:
            selector = f'window.mochi-v2.{self.material_css_class}'
            material = (f'{selector} .material-speech {{ background: {bubble.background}; '
                        f'color: {bubble.foreground}; border-color: {bubble.border}; }}\n'
                        f'{selector} .material-user {{ background: {bubble.user_background}; '
                        f'color: {bubble.foreground}; border-color: {bubble.border}; }}')
            self.material_provider.load_from_data(material.encode())


    def render_page(self, models=None):
        if models is not None:
            self.character_models = models
        if isinstance(self.page, LibraryPage):
            self.library_states[self.page.category] = self.page.view_state()
        if self.page:
            self.page.detach()
            self.page = None
        old_topline = self.compact_themes.get_parent()
        if isinstance(old_topline, Gtk.Box):
            old_topline.remove(self.compact_themes)
        self._clear(self.workspace_content)
        library = self.active_area == 'library'
        self.sidebar.set_visible(not self.compact and self.active_area == 'themes')
        self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.NEVER if library else Gtk.PolicyType.AUTOMATIC)
        self.workspace_content.set_vexpand(library)
        for key, button in self.rail_buttons.items():
            if key == self.active_area:
                button.add_css_class('active')
            else:
                button.remove_css_class('active')
        if self.active_area != 'themes':
            if self.active_area == 'library':
                self.page = LibraryPage(self.library_snapshot, self.state.route.library_tab, self.callbacks, self.layout, self.compact, self.library_job)
                self.workspace_content.append(self.page.root)
                self.page.set_width(self.get_width())
                self.page.restore_state(self.library_states.get(self.page.category, {}))
            else:
                page_type = ConnectionsPage if self.active_area == 'connections' else SettingsPage
                self.page = page_type(self.state, self.callbacks, self.layout, self.character_models, self.resources, self.compact)
                self.workspace_content.append(self.page.root)
                if hasattr(self.page, 'set_width'):
                    self.page.set_width(self.get_width())
            self.update_runtime()
            return
        theme = self.state.draft.value
        topline = styled(Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['md']), 'topline')
        topline.set_hexpand(True)
        self.compact_themes.set_visible(bool(self.compact))
        topline.append(self.compact_themes)
        title_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        title_box.set_hexpand(True)
        self.title_label = text(theme['name'], 'page-title')
        self.title_label.set_ellipsize(Pango.EllipsizeMode.END)
        self.title_label.set_tooltip_text(theme['name'])
        title_box.append(self.title_label)
        title_box.append(text('Theme workspace', 'subtitle'))
        topline.append(title_box)
        self.workspace_content.append(topline)
        tabs = styled(Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL,
                              spacing=self.layout['spacing']['sm']), 'tabs')
        self.tabs = {}
        for key, title in (('preview', 'Preview'), ('cast', 'Cast'), ('behavior', 'Behavior'),
                           ('bubble', 'Bubble'), ('features', 'Features')):
            button = Gtk.Button(label=_(title))
            button.connect('clicked', lambda _button, value=key: self.callbacks['tab'](value))
            if key == self.active_tab:
                button.add_css_class('active')
            tabs.append(button)
            self.tabs[key] = button
        self.workspace_content.append(tabs)
        page_type = PAGES.get(self.active_tab, PreviewPage)
        self.page = page_type(self.state, self.callbacks, self.layout, self.character_models, self.resources, self.compact)
        self.workspace_content.append(self.page.root)
        self.update_runtime()

    def update_theme_identity(self):
        if self.active_area == 'themes':
            self.title_label.set_text(self.state.draft.value['name'])
            self.title_label.set_tooltip_text(self.state.draft.value['name'])


    def show_error(self, error):
        summary, detail = error_parts(error)
        brief = detail.replace('\n', ' ').strip()
        self.error.set_text(summary + (' — '+brief[:160]+('…' if len(brief) > 160 else '') if brief else ''))
        self.error.set_tooltip_text(detail[:8192] if detail else None)
        self.error.set_visible(True)
        self.error_detail_text.get_buffer().set_text(detail[:8192] + ('\n…' if len(detail) > 8192 else ''))
        self.error_details.set_expanded(False)
        self.error_details.set_visible(bool(detail))


    def clear_error(self):
        self.error.set_visible(False)
        self.error_details.set_visible(False)


    def dispose_style(self):
        self.undo.dispose()
        if self.page:
            self.page.detach()
            self.page = None
        Gtk.StyleContext.remove_provider_for_display(Gdk.Display.get_default(), self.material_provider)
        if self.surface_width_handler is not None:
            self.get_surface().disconnect(self.surface_width_handler)
            self.surface_width_handler = None
        Gtk.StyleContext.remove_provider_for_display(Gdk.Display.get_default(), self.provider)


    def _watch_surface_width(self, *_args):
        surface = self.get_surface()
        self.surface_width_handler = surface.connect('notify::width', self._width_changed)
        self._set_responsive(surface.get_width())


    def _width_changed(self, surface, *_args):
        self._set_responsive(surface.get_width())


    def _set_responsive(self, width):
        # Switch before the full rail/sidebar/workspace minimum can pin the
        # window above the requested narrow width.
        compact = width <= self.layout['compactBreakpoint']
        if compact != self.compact:
            self.compact = compact
            self.root.add_css_class('compact') if compact else self.root.remove_css_class('compact')
            self.sidebar.set_visible(not compact and self.active_area == 'themes')
            self.rail.set_size_request(self.layout['railCompactWidth'] if compact else self.layout['railWidth'], -1)
            self.sidebar.set_size_request(self.layout['themeSidebarMediumWidth'] if width <= self.layout['mediumBreakpoint'] else self.layout['themeSidebarWidth'], -1)
            self.commandbar.responsive(compact)
            self.banner.responsive(compact)
            self.repair.responsive(compact)
            self.compact_themes.set_visible(compact and self.active_area == 'themes')
            if self.page:
                self.page.responsive(compact)
        elif not compact:
            self.sidebar.set_size_request(self.layout['themeSidebarMediumWidth'] if width <= self.layout['mediumBreakpoint'] else self.layout['themeSidebarWidth'], -1)

        if self.page and hasattr(self.page, 'set_width'):
            self.page.set_width(width)


    def show_banner(self, *args):
        return self.banner.show_banner(*args)

    def hide_banner(self, *args):
        return self.banner.hide_banner(*args)

    def show_undo(self, *args):
        return self.undo.show_undo(*args)

    def hide_undo(self, *args):
        return self.undo.hide_undo(*args)

    def show_notice(self, *args):
        return self.undo.show_notice(*args)

    def update_runtime(self, info=None):
        if info is not None:
            self.repair.update(info.get('repairs', ()) if self.active_area == 'themes' else ())
        return self.commandbar.update_runtime(info)

    def update_preview_size(self, *args):
        if self.page and hasattr(self.page, 'update_preview_size'):
            self.page.update_preview_size(*args)

    def select_preview_character(self, *args):
        if self.page and hasattr(self.page, 'select_preview_character'):
            self.page.select_preview_character(*args)

    def update_translation(self, *args):
        if self.page and hasattr(self.page, 'update_translation'):
            self.page.update_translation(*args)
