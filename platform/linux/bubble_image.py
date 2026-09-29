"""A fixed rectangular material image, rendered by GTK at display scale."""
import gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk


def material_image(path, width, height):
    widget = Gtk.Box()
    widget.set_size_request(width,height)
    widget.get_style_context().add_class('mochi-material-image')
    style = Gtk.CssProvider()
    # CSS backgrounds preserve rectangular geometry. Gtk.Image file icons
    # request a square slot, adding unwanted whitespace to wide frame sprites.
    rule = ".mochi-material-image { background-image: url('"+path.resolve().as_uri()+"'); background-size: contain; background-repeat: no-repeat; background-position: center; }"
    style.load_from_data(rule.encode())
    widget.get_style_context().add_provider(style,Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
    return widget
