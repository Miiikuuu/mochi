"""Presentation primitives shared by the GTK4 shell, pages and components."""
import gi
gi.require_version('Gtk', '4.0')
gi.require_version('GdkPixbuf', '2.0')
from gi.repository import Gdk, GdkPixbuf, GLib, GObject, Gtk, Pango
from manager_i18n import _, error_parts
from manager_preview_geometry import preview_geometry


def styled(widget, name):
    widget.add_css_class(name)
    return widget


def text(value, style=None):
    label = Gtk.Label(label=_(value), xalign=0)
    if style:
        label.add_css_class(style)
    if style in ('section-hint', 'field-help'):
        label.set_wrap(True)
        label.set_wrap_mode(Pango.WrapMode.WORD_CHAR)
        label.set_max_width_chars(32)
    return label


def clear(box):
    child = box.get_first_child()
    while child:
        following = child.get_next_sibling()
        box.remove(child)
        child = following


class Page:
    def __init__(self, state, callbacks, layout, models, resources, compact):
        self.state, self.layout = state, layout
        self.callbacks = {key: (lambda *args, operation=callback:
                                operation(*args) if not self.disposed else None)
                          for key, callback in callbacks.items()}
        self.character_models = models
        self.feature_models, self.bubble_model, self.translation_options, self.bubble_choices = resources
        self.compact = compact
        self.disposed = False
        self.workspace_content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.root = self.workspace_content

    def detach(self):
        self.dispose()

    def dispose(self):
        self.disposed = True

    def responsive(self, compact):
        self.compact = compact

    @staticmethod
    def _clear(box):
        clear(box)


def thumbnail_pixbuf(thumbnail):
    return GdkPixbuf.Pixbuf.new_from_bytes(GLib.Bytes.new(thumbnail.pixels),
        GdkPixbuf.Colorspace.RGB, True, 8, thumbnail.width, thumbnail.height, thumbnail.rowstride)
