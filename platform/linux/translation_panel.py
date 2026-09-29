"""GTK translation management view. Task data belongs to its controller."""
import time
from pathlib import Path
import gi
gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
from gi.repository import Gdk, Gio, Gtk, Pango
from translation_controller import LANGUAGES

from mochi_paths import DATA as ROOT

def contents(view):
    buffer = view.get_buffer()
    return buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)

def combo(values):
    widget = Gtk.ComboBoxText()
    for key, title in values.items():
        widget.append(str(key), title)
    widget.set_active(0)
    return widget

def set_icon(button, name, hint):
    if getattr(button, '_icon_name', None) != name:
        if name == 'mochi-translate-symbolic':
            icon = Gio.FileIcon.new(Gio.File.new_for_path(str(ROOT/'assets/icons/translate-symbolic.svg')))
            button.set_image(Gtk.Image.new_from_gicon(icon, Gtk.IconSize.MENU))
        else:
            button.set_image(Gtk.Image.new_from_icon_name(name, Gtk.IconSize.MENU))
        button.set_always_show_image(True)
        button._icon_name = name
    button.set_tooltip_text(hint)
    button.get_accessible().set_name(hint)

def icon_button(name, hint, toggle=False):
    button = Gtk.ToggleButton() if toggle else Gtk.Button()
    button.set_relief(Gtk.ReliefStyle.NONE)
    button.get_style_context().add_class('mochi-icon-button')
    set_icon(button, name, hint)
    return button

class SourceSection(Gtk.ScrolledWindow):
    """The existing source foldout, controlled by a toolbar icon."""
    def __init__(self, text, toggle, resized):
        super().__init__()
        self.toggle, self.resized = toggle, resized
        self.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.set_size_request(-1, 80)
        self.set_no_show_all(True)
        self.add(text)
        text.show()
        toggle.connect('toggled', lambda button: self.set_expanded(button.get_active()))

    def set_expanded(self, expanded):
        if self.get_visible() == expanded and self.toggle.get_active() == expanded:
            return
        self.set_visible(expanded)
        self.toggle.set_active(expanded)
        hint = '收起原文' if expanded else '查看或编辑原文'
        self.toggle.set_tooltip_text(hint)
        self.toggle.get_accessible().set_name(hint)
        self.resized()

class TranslationPanel:
    def __init__(self, controller, close, resized, themes=None, select_theme=None):
        self.controller, self.on_close, self.resized = controller, close, resized
        self.themes, self.select_theme = themes, select_theme
        self.theme_restoring = False
        self.theme_picker = None
        self.restoring = True
        self.model_names = controller.model_names
        self.build_window()
        self.window.connect('notify::visible', lambda *_: self.sync_spinner())
        self.source.get_buffer().connect('changed', self.source_changed)
        self.language.connect('changed', self.settings_changed)
        self.model.connect('changed', self.settings_changed)
        self.fast.connect('toggled', self.settings_changed)
        controller.subscribe(self.render)
        self.render()

    def build_window(self):
        self.window = Gtk.Window(title='Mochi · 翻译管理')
        self.window.set_wmclass('mochi-bubble', 'MochiBubble')
        self.window.set_decorated(False)
        self.window.set_resizable(False)
        self.window.set_keep_above(True)
        self.window.set_skip_taskbar_hint(True)
        self.window.set_skip_pager_hint(True)
        self.window.set_focus_on_map(False)
        self.window.set_type_hint(Gdk.WindowTypeHint.UTILITY)
        self.window.get_style_context().add_class('mochi-bubble')
        self.window.get_style_context().add_class('mochi-translation')
        visual = self.window.get_screen().get_rgba_visual()
        if visual:
            self.window.set_visual(visual)
        self.window.connect('delete-event', lambda *_: self.on_close())
        self.window.connect('key-press-event', self.key)
        self.window.connect('size-allocate', lambda *_: self.resized())
        style = Gtk.CssProvider()
        style.load_from_path(str(ROOT/'assets/bubbles/default/style.css'))
        Gtk.StyleContext.add_provider_for_screen(self.window.get_screen(), style, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        card.get_style_context().add_class('mochi-card')
        card.set_size_request(320, -1)
        self.stack = Gtk.Stack()
        self.stack.set_homogeneous(False)
        self.stack.set_transition_type(Gtk.StackTransitionType.CROSSFADE)
        self.stack.set_transition_duration(150)
        self.window.add(self.stack)
        self.stack.add_named(card, 'translation')
        header = Gtk.Box(spacing=8)
        self.original_button = icon_button('document-edit-symbolic', '查看或编辑原文', toggle=True)
        header.pack_start(self.original_button, False, False, 0)
        close = icon_button('window-close-symbolic', '关闭翻译面板并取消任务')
        close.connect('clicked', lambda *_: self.on_close())
        header.pack_end(close, False, False, 0)
        self.settings = icon_button('preferences-system-symbolic', '设置')
        self.settings.connect('clicked', self.open_settings)
        header.pack_end(self.settings, False, False, 0)
        card.pack_start(header, False, False, 0)
        self.build_settings()
        self.source = Gtk.TextView(wrap_mode=Gtk.WrapMode.WORD_CHAR)
        self.source.get_accessible().set_name('原文')
        self.source.set_tooltip_text('输入要翻译的文字')
        self.source.get_style_context().add_class('mochi-source')
        self.original = SourceSection(self.source, self.original_button, lambda: self.window.resize(1, 1))
        card.pack_start(self.original, False, False, 0)
        self.status = Gtk.Label(xalign=0)
        self.status.set_line_wrap(True)
        self.status.set_max_width_chars(40)
        self.status.get_style_context().add_class('mochi-status')
        self.status.set_no_show_all(True)
        card.pack_start(self.status, False, False, 0)
        self.result = Gtk.TextView(editable=False, wrap_mode=Gtk.WrapMode.WORD_CHAR, cursor_visible=False)
        self.result.get_accessible().set_name('译文')
        self.result.set_left_margin(0)
        self.result.set_right_margin(0)
        self.result.set_top_margin(4)
        self.result.set_bottom_margin(4)
        self.result.get_style_context().add_class('mochi-result')
        self.scroll = Gtk.ScrolledWindow()
        self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.scroll.set_size_request(-1, 36)
        self.scroll.set_no_show_all(True)
        self.scroll.add(self.result)
        card.pack_start(self.scroll, True, True, 0)
        buttons = Gtk.Box(spacing=8)
        self.spinner = Gtk.Spinner()
        self.spinner.set_no_show_all(True)
        self.spinner.set_tooltip_text('正在翻译')
        self.spinner.get_accessible().set_name('正在翻译')
        buttons.pack_start(self.spinner, False, False, 0)
        self.cancel = icon_button('media-playback-stop-symbolic', '取消翻译')
        self.cancel.connect('clicked', lambda *_: self.controller.cancel())
        self.copy = icon_button('edit-copy-symbolic', '复制译文')
        self.copy.connect('clicked', self.copy_text)
        self.retry = icon_button('mochi-translate-symbolic', '翻译')
        self.retry.connect('clicked', lambda *_: self.translate())
        for widget in [self.cancel, self.copy, self.retry]:
            widget.set_no_show_all(True)
        buttons.pack_start(self.copy, False, False, 0)
        buttons.pack_end(self.retry, False, False, 0)
        buttons.pack_end(self.cancel, False, False, 0)
        card.pack_start(buttons, False, False, 0)
        self.set_busy(False)
        self.copy.set_sensitive(False)


    def build_settings(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        page.get_style_context().add_class('mochi-card')
        page.set_size_request(320, -1)
        header = Gtk.Box(spacing=8)
        self.settings_back = icon_button('go-previous-symbolic', '返回翻译')
        self.settings_back.connect('clicked', lambda *_: self.settings_page(False))
        header.pack_start(self.settings_back, False, False, 0)
        close = icon_button('window-close-symbolic', '关闭翻译面板并取消任务')
        close.connect('clicked', lambda *_: self.on_close())
        header.pack_end(close, False, False, 0)
        page.pack_start(header, False, False, 0)
        if self.themes:
            self.theme_picker = Gtk.ComboBoxText()
            group = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
            row = Gtk.Box(spacing=12)
            caption = Gtk.Label(label='外观', xalign=0)
            caption.set_size_request(36, -1)
            row.pack_start(caption, False, False, 0)
            row.pack_start(self.theme_picker, True, True, 0)
            group.pack_start(row, False, False, 0)
            self.theme_status = Gtk.Label(xalign=0)
            self.theme_status.get_style_context().add_class('mochi-status')
            self.theme_status.set_no_show_all(True)
            group.pack_start(self.theme_status, False, False, 0)
            page.pack_start(group, False, False, 0)
            self.sync_theme()
            self.theme_picker.connect('changed', self.theme_changed)
        self.language, self.model = combo(LANGUAGES), combo(self.model_names)
        self.refresh = icon_button('view-refresh-symbolic', '刷新模型列表')
        self.refresh.connect('clicked', lambda *_: self.refresh_models())
        for label, widget in [('语言', self.language), ('模型', self.model)]:
            group = Gtk.Box(spacing=12)
            caption = Gtk.Label(label=label, xalign=0)
            caption.set_size_request(36, -1)
            group.pack_start(caption, False, False, 0)
            group.pack_start(widget, True, True, 0)
            if widget is self.model:
                group.pack_end(self.refresh, False, False, 0)
            page.pack_start(group, False, False, 0)
        self.catalog_status = Gtk.Label(xalign=0)
        self.catalog_status.set_line_wrap(True)
        self.catalog_status.set_max_width_chars(28)
        self.catalog_status.get_style_context().add_class('mochi-status')
        self.catalog_status.set_no_show_all(True)
        page.pack_start(self.catalog_status, False, False, 0)
        self.fast = Gtk.CheckButton(label='Fast')
        self.fast.get_style_context().add_class('mochi-fast')
        self.fast.set_tooltip_text('加速当前模型，开启后额度消耗更高')
        page.pack_start(self.fast, False, False, 0)
        self.stack.add_named(page, 'settings')


    def settings_page(self, opened):
        self.stack.set_visible_child_name('settings' if opened else 'translation')
        self.sync_spinner()
        self.window.resize(1, 1)
        self.resized()

    def open_settings(self, *_):
        self.sync_theme()
        self.settings_page(True)
        if self.controller.catalog is None or time.monotonic() - self.controller.catalog_time > 300:
            self.controller.refresh_models()

    def sync_theme(self):
        if not self.theme_picker:return
        self.theme_restoring = True
        self.theme_picker.remove_all()
        for key,name in self.themes.choices.items():self.theme_picker.append(key,name)
        self.theme_picker.set_active_id(self.themes.selected)
        self.theme_restoring = False

    def theme_changed(self, *_):
        if self.theme_restoring:return
        try:
            self.select_theme(self.theme_picker.get_active_id())
            self.theme_status.hide()
        except Exception as error:
            self.sync_theme()
            self.theme_status.set_text('气泡素材无法加载，已保留当前外观。')
            self.theme_status.show()
            print('Bubble theme selection failed:',error,flush=True)

    def refresh_models(self):
        self.controller.refresh_models()

    def settings_changed(self, widget):
        if self.restoring:
            return
        catalog = self.controller.catalog
        if widget is self.model and catalog is not None and not catalog.get(self.model.get_active_id(), {}).get('fast_tier'):
            self.restoring = True
            self.fast.set_active(False)
            self.restoring = False
        self.controller.configure(self.language.get_active_id(), self.model.get_active_id(), self.fast.get_active())

    def source_changed(self, _buffer):
        if not self.restoring:
            self.controller.set_source(contents(self.source))

    def update_fast(self):
        catalog = self.controller.catalog
        supported = catalog is not None and bool(catalog.get(self.model.get_active_id(), {}).get('fast_tier'))
        self.fast.set_sensitive((supported or self.fast.get_active()) and not self.controller.busy)
        if catalog is None:
            hint = '读取模型列表后可设置 Fast。'
        elif not supported:
            hint = '当前模型不提供 Fast，请关闭或选择其他模型。'
        elif self.fast.get_active():
            hint = 'Fast 已开启，额度消耗更高。'
        else:
            hint = '开启后速度更快，额度消耗更高。'
        self.fast.set_tooltip_text(hint)

    def sync_spinner(self):
        active = (self.controller.busy and self.window.get_visible()
                  and self.stack.get_visible_child_name() == 'translation')
        self.spinner.set_visible(active)
        if active:
            self.spinner.start()
        else:
            self.spinner.stop()

    def set_busy(self, busy):
        self.language.set_sensitive(not busy)
        self.model.set_sensitive(not busy)
        self.source.set_editable(not busy)
        self.retry.set_sensitive(not busy)
        self.cancel.set_sensitive(busy)
        self.update_fast()

    def render(self):
        c = self.controller
        self.restoring = True
        names = c.model_names
        if names != self.model_names:
            self.model.remove_all()
            for key, name in names.items():
                self.model.append(key, name)
            self.model_names = names
        self.model.set_active_id(c.preferences['model'])
        self.language.set_active_id(c.preferences['target'])
        self.fast.set_active(c.preferences['fast'])
        for widget, text in ((self.source, c.source), (self.result, c.text)):
            if contents(widget) != text:
                widget.get_buffer().set_text(text)
        self.status.set_text(c.status)
        self.status.set_visible(bool(c.status) and c.state.value == 'error')
        self.result.set_tooltip_text(c.status or None)
        self.result.show()
        if c.text:
            layout = self.result.create_pango_layout(c.text)
            layout.set_width(296 * Pango.SCALE)
            layout.set_wrap(Pango.WrapMode.WORD_CHAR)
            self.scroll.set_size_request(-1, max(36, min(180, layout.get_pixel_size()[1] + 8)))
        self.scroll.set_visible(bool(c.text))
        self.catalog_status.set_text(c.catalog_status)
        self.catalog_status.set_visible(c.catalog_job is None and bool(c.catalog_status) and
                                       not c.catalog_status.startswith(('已同步 ', '从已登录', '正在读取')))
        self.refresh.set_tooltip_text(c.catalog_status or '刷新模型列表')
        self.refresh.set_sensitive(c.catalog_job is None)
        self.copy.set_sensitive(bool(c.text))
        set_icon(self.copy, 'object-select-symbolic' if c.copied else 'edit-copy-symbolic',
                 '已复制' if c.copied else '复制译文')
        self.copy.set_visible(bool(c.text) and not c.busy)
        self.cancel.set_visible(c.busy)
        set_icon(self.retry, 'mochi-translate-symbolic',
                 '重新翻译' if c.text else '重试' if c.state.value == 'error' else '翻译')
        self.retry.set_visible(not c.busy)
        self.set_busy(c.busy)
        self.restoring = False
        self.sync_spinner()
        self.window.resize(1, 1)

    def translate(self):
        try:
            self.controller.start()
        except ValueError:
            pass  # The controller published the validation status to this view.

    def copy_text(self, *_):
        self.controller.action(self.controller.message_id, 'copy')

    def show(self):
        self.controller.prepare_panel()
        if not self.controller.source:
            self.original.set_expanded(True)
        self.settings_page(False)
        self.window.show_all()
        self.render()
        self.window.realize()

    def key(self, _widget, event):
        if event.keyval == Gdk.KEY_Escape:
            if self.stack.get_visible_child_name() == 'settings':
                self.settings_page(False)
            else:
                self.on_close()
            return True
        return False

    def inspect(self):
        controls = {}
        for name in ['cancel', 'copy', 'retry', 'settings', 'settings_back', 'refresh', 'fast', 'model', 'language', 'theme_picker', 'original_button']:
            widget = getattr(self, name)
            if widget is None:continue
            xy = widget.translate_coordinates(self.window, 0, 0)
            size = widget.get_allocation()
            controls[name] = [*xy, size.width, size.height] if xy and widget.get_mapped() else None
        return controls
