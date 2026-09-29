"""Small GTK building blocks for the supplied Mochi UI design."""
from functools import lru_cache
from concurrent.futures import ThreadPoolExecutor
import weakref
from manager_resources import ThumbnailCache
from pathlib import Path
from gi.repository import GdkPixbuf, Gio, GLib, Gtk, Pango
from manager_i18n import _
from mochi_paths import DATA as ROOT


def box(vertical=False, spacing=12):
    return Gtk.Box(orientation=Gtk.Orientation.VERTICAL if vertical else Gtk.Orientation.HORIZONTAL, spacing=spacing)


def css(widget, name):
    widget.get_style_context().add_class(name)
    return widget


def label(text='', style=None, expand=False):
    widget = Gtk.Label(label=_(text), xalign=0)
    widget.set_ellipsize(Pango.EllipsizeMode.END)
    widget.set_hexpand(expand)
    if style:
        css(widget, style)
    return widget


@lru_cache(maxsize=192)
def icon_data(name, color):
    svg = (ROOT/'assets/manager/icons'/(name+'-symbolic.svg')).read_text()
    return Gio.BytesIcon.new(GLib.Bytes.new(svg.replace('#2e3436', color).encode()))


class Icon(Gtk.Image):
    """Stroke icons inherit GTK foreground and remain sharp at any desktop scale."""
    def __init__(self, name, size):
        super().__init__()
        self.name, self.size = name, size
        self.current = None
        self.set_pixel_size(size)
        self.set_halign(Gtk.Align.CENTER)
        self.set_valign(Gtk.Align.CENTER)
        self.connect('style-updated', self.refresh_icon)
        self.connect('state-flags-changed', self.refresh_icon)
        self.refresh_icon()

    def set_symbol(self, name):
        if self.name != name:
            self.name = name
            self.refresh_icon()

    def refresh_icon(self, *_):
        color = self.get_style_context().get_color(self.get_state_flags()).to_string()
        key = self.name, color
        if self.current != key:
            self.current = key
            self.set_from_gicon(icon_data(*key), Gtk.IconSize.BUTTON)


def icon(name, size=18):
    return Icon(name, size)


def icon_button(name, hint, text=None):
    button = css(Gtk.Button(), 'icon-button' if text is None else 'text-button')
    if text is None:
        button.add(icon(name))
    else:
        content = box(spacing=8)
        content.pack_start(icon(name, 16), False, False, 0)
        content.pack_start(label(text), False, False, 0)
        button.add(content)
    button.set_tooltip_text(_(hint))
    button.get_accessible().set_name(_(hint))
    return button


NAMES = {'shanbi': 'Shanbi', 'angelina_wish': 'Angelina', 'jacinta': 'Jacinta',
         'timeslot': 'Timeslot', 'mutsumi': 'Wakaba Mutsumi'}


_portrait_worker = ThreadPoolExecutor(max_workers=1, thread_name_prefix='mochi-portraits')
_portrait_cache = ThumbnailCache()


def portrait(key, entry, height=108, max_width=150):
    name = 'angelina' if key == 'angelina_wish' else key
    source = ROOT/'assets/manager/portraits'/(name+'.png') if key in NAMES else Path(entry['preview'] or '')
    widget = Gtk.Image()
    widget.set_size_request(max_width, height)
    reference = weakref.ref(widget)
    def complete(thumbnail):
        current = reference()
        if current is None or not current.get_parent() or thumbnail is None:
            return False
        picture = GdkPixbuf.Pixbuf.new_from_bytes(GLib.Bytes.new(thumbnail.pixels),
            GdkPixbuf.Colorspace.RGB, True, 8, thumbnail.width, thumbnail.height, thumbnail.rowstride)
        factor = min(max_width/picture.get_width(), height/picture.get_height())
        current.set_from_pixbuf(picture.scale_simple(max(1, round(picture.get_width()*factor)),
            max(1, round(picture.get_height()*factor)), GdkPixbuf.InterpType.BILINEAR))
        return False
    def load():
        thumbnail = _portrait_cache.load(source)
        GLib.idle_add(complete, thumbnail)
    _portrait_worker.submit(load)
    return widget


def field(title, control, symbol=None):
    row = css(box(spacing=16), 'field')
    left = box(spacing=8)
    if symbol:
        left.pack_start(css(icon(symbol, 16), 'muted'), False, False, 0)
    left.pack_start(label(title), True, True, 0)
    row.pack_start(left, True, True, 0)
    control.set_valign(Gtk.Align.CENTER)
    control.get_accessible().set_name(title)
    row.pack_end(control, False, False, 0)
    return row


def combo(choices, selected):
    widget = Gtk.ComboBoxText()
    for key, name in choices.items():
        widget.append(key, _(name))
    widget.set_active_id(selected)
    return widget
