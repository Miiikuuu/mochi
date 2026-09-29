"""Small host-rendered management view for declarative process plugins."""
from gi.repository import Gdk, Gtk, Pango
from plugin_catalog import ROOT
from reply_composer import ReplyComposer


class PluginPanel:
    def __init__(self, name, close, action, resized, reply=None):
        self.action, self.resized = action, resized
        self.window = Gtk.Window(title='Mochi · '+name)
        self.window.set_wmclass('mochi-plugin-panel', 'MochiPluginPanel')
        self.window.set_decorated(False)
        self.window.set_resizable(False)
        self.window.set_keep_above(True)
        self.window.set_skip_taskbar_hint(True)
        self.window.set_skip_pager_hint(True)
        self.window.set_focus_on_map(False)
        self.window.set_type_hint(Gdk.WindowTypeHint.UTILITY)
        self.window.get_style_context().add_class('mochi-bubble')
        self.window.get_style_context().add_class('mochi-translation')
        self.window.connect('delete-event', lambda *_: close())
        self.window.connect('key-press-event', lambda _, event: close() if event.keyval == Gdk.KEY_Escape else False)
        self.window.connect('size-allocate', lambda *_: resized())
        self.provider = Gtk.CssProvider()
        self.provider.load_from_path(str(ROOT/'assets/bubbles/default/style.css'))
        self.window.get_style_context().add_provider(self.provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        card.get_style_context().add_class('mochi-card')
        card.get_style_context().add_provider(self.provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        card.set_size_request(320, -1)
        self.window.add(card)
        header = Gtk.Box(spacing=10)
        self.title = Gtk.Label(label=name, xalign=0)
        self.title.set_ellipsize(Pango.EllipsizeMode.END)
        self.title.set_max_width_chars(24)
        header.pack_start(self.title, True, True, 0)
        button = Gtk.Button.new_from_icon_name('window-close-symbolic', Gtk.IconSize.BUTTON)
        button.set_relief(Gtk.ReliefStyle.NONE)
        button.set_tooltip_text('关闭')
        button.connect('clicked', lambda *_: close())
        header.pack_end(button, False, False, 0)
        card.pack_start(header, False, False, 0)
        self.text = Gtk.TextView(editable=False, cursor_visible=False, wrap_mode=Gtk.WrapMode.WORD_CHAR)
        self.text.set_size_request(290, 96)
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_min_content_height(96)
        scroll.set_max_content_height(280)
        scroll.set_propagate_natural_height(True)
        scroll.add(self.text)
        card.pack_start(scroll, True, True, 0)
        self.composer = ReplyComposer(reply or (lambda _: None), resized)
        card.pack_start(self.composer, False, False, 0)
        self.status = Gtk.Label(xalign=0)
        self.status.set_line_wrap(True)
        self.status.set_max_width_chars(36)
        self.status.set_no_show_all(True)
        card.pack_start(self.status, False, False, 0)
        self.buttons = Gtk.Grid(column_spacing=8, row_spacing=8)
        card.pack_start(self.buttons, False, False, 0)
        self.controls = {}
        self.actions = None
        self.show_status = False

    def render(self, content):
        self.title.set_text(content.title)
        self.text.get_buffer().set_text(content.text)
        self.status.set_text(content.status)
        self.composer.render(content.reply)
        self.show_status = content.show_status
        self.status.set_visible(content.show_status and bool(content.status))
        if self.actions != content.actions:
            self.actions = content.actions
            for child in self.buttons.get_children():
                child.destroy()
            self.controls = {}
            for index, item in enumerate(content.actions):
                button = Gtk.Button(label=item.label)
                button.get_child().set_ellipsize(Pango.EllipsizeMode.END)
                button.get_child().set_max_width_chars(8)
                button.set_sensitive(item.enabled)
                button.set_tooltip_text(item.tooltip or item.label)
                button.connect('clicked', lambda _, key=item.id: self.action(key))
                self.buttons.attach(button, index % 3, index // 3, 1, 1)
                self.controls[item.id] = button
            self.buttons.show_all()
        self.resized()

    def show(self):
        self.window.show_all()
        self.status.set_visible(self.show_status and bool(self.status.get_text()))

    def sync_theme(self):
        pass

    def inspect(self):
        result = {}
        for key, button in {**self.controls, 'reply_input': self.composer.entry, 'reply_send': self.composer.button}.items():
            point = button.translate_coordinates(self.window, 0, 0)
            rect = button.get_allocation()
            if point:
                result[key] = [*point, rect.width, rect.height]
        return result
