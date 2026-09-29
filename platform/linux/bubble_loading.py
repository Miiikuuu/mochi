"""Material-owned loading animation; has no knowledge of message producers."""
import json
import logging
from pathlib import Path
import time

import gi
gi.require_version('Gtk', '3.0')
from gi.repository import GdkPixbuf, GLib, Gtk
from bubble_image import material_image


def load_spec(assets):
    """An optional frame sequence, confined to the bubble material directory."""
    assets = Path(assets).resolve()
    path = assets/'loading.json'
    if not path.exists():
        return None  # Older materials keep their existing status text.
    if path.stat().st_size > 16384:
        raise ValueError('loading.json exceeds 16 KiB')
    spec = json.loads(path.read_text())
    if not isinstance(spec, dict) or spec.get('version') != 1 or type(spec.get('enabled', True)) is not bool:
        raise ValueError('Invalid loading animation version or enabled flag')
    if not spec.get('enabled', True):
        return None
    for key, low, high in [('size', 12, 96), ('frame_ms', 40, 2000), ('gap', 0, 24)]:
        if type(spec.get(key)) is not int or not low <= spec[key] <= high:
            raise ValueError('Invalid loading animation '+key)
    for key, high in [('width', 160), ('height', 96)]:
        spec.setdefault(key, spec['size'])
        if type(spec[key]) is not int or not 12 <= spec[key] <= high:
            raise ValueError('Invalid loading animation '+key)
    if spec.get('placement', 'status') not in ('status', 'footer'):
        raise ValueError('Invalid loading animation placement')
    caption = spec.get('caption')
    if caption is not None:
        if (not isinstance(caption, dict) or not isinstance(caption.get('text'), str) or len(caption['text']) > 40
                or type(caption.get('x')) is not int or type(caption.get('width')) is not int
                or not 0 <= caption['x'] < spec['width'] or not 1 <= caption['width'] <= spec['width']-caption['x']):
            raise ValueError('Invalid loading caption')
    if not isinstance(spec.get('frames'), list) or not 1 <= len(spec['frames']) <= 60:
        raise ValueError('Loading animation needs 1–60 frames')
    frames = []
    names = spec['frames'] + ([spec['idle_frame']] if spec.get('idle_frame') else [])
    for name in names:
        if not isinstance(name, str) or Path(name).is_absolute():
            raise ValueError('Loading frames must use relative material paths')
        frame = (assets/name).resolve()
        if not frame.is_relative_to(assets) or frame.suffix.lower() not in ('.png', '.svg'):
            raise ValueError('Loading frames must be PNG/SVG files inside the material')
        if frame.stat().st_size > 256*1024:
            raise ValueError('Loading frame exceeds 256 KiB')
        frames.append(frame)
    return {**spec, 'frames': tuple(frames[:len(spec['frames'])]),
            'idle_frame': frames[-1] if spec.get('idle_frame') else None}


class LoadingAnimation(Gtk.Stack):
    def __init__(self, assets, static=False):
        super().__init__()
        self.set_no_show_all(True)
        self.set_transition_type(Gtk.StackTransitionType.NONE)
        self.set_halign(Gtk.Align.CENTER)
        self.set_valign(Gtk.Align.CENTER)
        self.get_style_context().add_class('mochi-speech-loading')
        self.spec = None
        self.frames = []
        self.idle = None
        self.source = None
        self.frame = 0
        self.loading = False
        self.started = 0
        self.settings = self.get_settings()
        try:
            self.spec = load_spec(assets)
            if self.spec:
                if static:self.spec['frames'] = self.spec['frames'][:1]
                paths = [*self.spec['frames']]
                if self.spec['idle_frame']:paths.append(self.spec['idle_frame'])
                for index, path in enumerate(paths):
                    # Validate every frame up front; GTK scales its material
                    # background at the monitor's resolution.
                    GdkPixbuf.Pixbuf.new_from_file_at_scale(str(path), self.spec['width'], self.spec['height'], True)
                    image = material_image(path,self.spec['width'],self.spec['height'])
                    if self.spec.get('caption') and index < len(self.spec['frames']):
                        caption = self.spec['caption']
                        overlay = Gtk.Overlay()
                        overlay.add(image)
                        label = Gtk.Label(label=caption['text'], xalign=0)
                        label.get_style_context().add_class('mochi-loading-caption')
                        label.set_halign(Gtk.Align.START)
                        label.set_valign(Gtk.Align.CENTER)
                        label.set_margin_start(caption['x'])
                        label.set_size_request(caption['width'], -1)
                        overlay.add_overlay(label)
                        overlay.set_overlay_pass_through(label,True)
                        image = overlay
                        image.show_all()
                    image.show()
                    self.add_named(image, str(index))
                    if index < len(self.spec['frames']):self.frames.append(image)
                    else:self.idle = image
        except (OSError, ValueError, GLib.Error) as error:
            logging.warning('Bubble loading animation disabled: %s', error)
            for image in self.get_children():
                image.destroy()
            self.frames = []
            self.idle = None
            self.spec = None
        self.connect('map', lambda *_: self.sync())
        self.connect('unmap', lambda *_: self.stop())
        self.settings_handler = self.settings.connect('notify::gtk-enable-animations', lambda *_: self.sync())
        self.connect('destroy', self.destroyed)

    @property
    def enabled(self):
        return bool(self.frames)

    @property
    def gap(self):
        return self.spec['gap'] if self.spec else 8

    @property
    def placement(self):
        return self.spec.get('placement', 'status') if self.spec else 'status'

    def set_loading(self, loading, restart=False):
        if loading != self.loading or restart:
            self.stop()
            self.frame = 0
            if self.frames:
                self.set_visible_child(self.frames[0])
        self.loading = loading
        if not loading and self.idle:
            self.set_visible_child(self.idle)
        self.set_visible((loading and self.enabled) or self.idle is not None)
        self.sync()

    def sync(self):
        animate = (self.loading and self.get_mapped() and len(self.frames) > 1
                   and self.settings.get_property('gtk-enable-animations'))
        if animate and self.source is None:
            self.started = time.monotonic() - self.frame*self.spec['frame_ms']/1000
            self.source = GLib.timeout_add(self.spec['frame_ms'], self.advance)
        elif not animate:
            self.stop()

    def advance(self):
        frame = int((time.monotonic()-self.started)*1000/self.spec['frame_ms']) % len(self.frames)
        if frame != self.frame:
            self.frame = frame
            self.set_visible_child(self.frames[frame])
        return True

    def stop(self):
        if self.source is not None:
            GLib.source_remove(self.source)
            self.source = None

    def destroyed(self, *_):
        self.stop()
        self.settings.disconnect(self.settings_handler)

    def inspect(self):
        return {'enabled': self.enabled, 'loading': self.loading, 'visible': self.get_mapped(),
                'animating': self.source is not None, 'frame': self.frame, 'frames': len(self.frames),
                'size': self.spec['size'] if self.spec else None,
                'frame_ms': self.spec['frame_ms'] if self.spec else None}
