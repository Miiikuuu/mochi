"""Optional button artwork and styling owned by a conversation material."""
import json
from gi.repository import Gtk
from bubble_image import material_image


def apply_controls(assets, root, buttons):
    path = assets/'controls.json'
    if not path.exists():return None
    spec = json.loads(path.read_text())
    if not isinstance(spec,dict) or spec.get('version') != 1 or not isinstance(spec.get('icons'),dict):
        raise ValueError('Invalid bubble controls')
    size = spec.get('size',18)
    if type(size) is not int or not 12 <= size <= 32:raise ValueError('Invalid control icon size')
    files = {}
    for key,name in spec['icons'].items():
        if key not in buttons or not isinstance(name,str):raise ValueError('Invalid bubble control icon')
        file = (assets/name).resolve()
        if not file.is_relative_to(assets.resolve()) or file.suffix not in ('.svg','.png') or not file.is_file():
            raise ValueError('Control icon must be a local material image')
        files[key] = file
    for key,button in buttons.items():
        button.get_style_context().add_class('mochi-control-'+key)
        if key in files:
            child = button.get_child()
            if child:button.remove(child)
            icon = material_image(files[key],size,size)
            icon.set_halign(Gtk.Align.CENTER)
            icon.set_valign(Gtk.Align.CENTER)
            button.add(icon)
            icon.show()
    style = Gtk.CssProvider()
    style.load_from_path(str(assets/'controls.css'))
    def attach(widget):
        widget.get_style_context().add_provider(style,Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION+1)
        if isinstance(widget,Gtk.Container):
            for child in widget.get_children():attach(child)
    attach(root)
    return style
