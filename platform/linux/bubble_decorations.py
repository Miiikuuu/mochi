"""Declarative, noninteractive decorations from a bubble material."""
import json
from pathlib import Path
import gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk
from bubble_image import material_image


def load_decorations(assets):
    path = assets/'decorations.json'
    if not path.exists():return {'overlays': []}
    data = json.loads(path.read_text())
    if not isinstance(data, dict) or not isinstance(data.get('overlays', []), list) or len(data.get('overlays', [])) > 6:
        raise ValueError('Invalid bubble decorations')
    for item in [*data.get('overlays', []), *([data['header']] if data.get('header') else [])]:
        if not isinstance(item, dict) or not isinstance(item.get('file'), str):
            raise ValueError('Invalid decoration file')
        file = (assets/item['file']).resolve()
        if not file.is_relative_to(assets.resolve()) or file.suffix.lower() not in ('.svg', '.png') or not file.is_file():
            raise ValueError('Decoration must be a local material image')
        for name in ['width','height']:
            if type(item.get(name)) is not int or not 1 <= item[name] <= 96:
                raise ValueError('Invalid decoration size')
        for name in ['x','y']:
            if type(item.get(name, 0)) is not int or not 0 <= item.get(name, 0) <= 160:
                raise ValueError('Invalid decoration offset')
        if item.get('align', 'start') not in ('start', 'end'):
            raise ValueError('Invalid decoration alignment')
    return data


def image(assets, item, scale=1):
    widget = material_image(assets/item['file'],round(item['width']*scale),round(item['height']*scale))
    widget.set_halign(Gtk.Align.END if item.get('align') == 'end' else Gtk.Align.START)
    widget.set_valign(Gtk.Align.START)
    return widget


def decorate(assets, overlay, header, scale=1):
    data = load_decorations(assets)
    header_image = None
    if data.get('header'):
        header_image = image(assets, data['header'], scale)
        header_image.set_valign(Gtk.Align.CENTER)
        header_image.set_no_show_all(True)
        header.pack_start(header_image, False, False, 0)
    for item in data.get('overlays', []):
        widget = image(assets, item, scale)
        if item.get('align') == 'end':widget.set_margin_end(round(item.get('x', 0)*scale))
        else:widget.set_margin_start(round(item.get('x', 0)*scale))
        widget.set_margin_top(round(item.get('y', 0)*scale))
        overlay.add_overlay(widget)
        overlay.set_overlay_pass_through(widget, True)
    return header_image
