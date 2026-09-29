"""Small, shared vector icons for the function menu and management panels."""
from pathlib import Path

import gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gio, Gtk

from mochi_paths import DATA
DIRECTORY = DATA / 'assets/icons/line'


def icon_image(name, size=18):
    # These are regular SVGs: GTK's symbolic recoloring discards SVG strokes.
    icon = Gio.FileIcon.new(Gio.File.new_for_path(str(DIRECTORY / (name + '.svg'))))
    image = Gtk.Image.new_from_gicon(icon, Gtk.IconSize.MENU)
    image.set_pixel_size(size)
    return image


def set_button_icon(button, name, hint):
    if getattr(button, '_icon_name', None) != name:
        button.set_image(icon_image(name))
        button.set_always_show_image(True)
        button._icon_name = name
    button.set_tooltip_text(hint)
    button.get_accessible().set_name(hint)
