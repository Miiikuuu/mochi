"""Main-character function entries. All coordinates here are GTK logical pixels.

The desktop supplies its existing pointer sample; hidden menus have no timers.
Only icon windows accept input, never the transparent space between them.
"""
import math
import time
from pathlib import Path

import gi
gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
from gi.repository import Gdk, Gio, GLib, Gtk

from mochi_paths import DATA as ROOT
ENTRY_STYLE = None


def entry_style():
    global ENTRY_STYLE
    if ENTRY_STYLE is None:
        ENTRY_STYLE = Gtk.CssProvider()
        ENTRY_STYLE.load_from_path(str(ROOT/'assets/icons/function-menu.css'))
    return ENTRY_STYLE

SIZE = 72
LEAVE_MS = 280
ENTER_SECONDS = .42
EXIT_SECONDS = .22
STAGGER_SECONDS = .075


def contains(rect, point, pad=0):
    x, y, w, h = rect
    return x-pad <= point[0] <= x+w+pad and y-pad <= point[1] <= y+h+pad


def arc_positions(anchor, count, work):
    return ArcLayout(anchor, count, work).targets


class ArcLayout:
    """Bottom-to-top slots, approached along the same lower-left ellipse."""
    def __init__(self, anchor, count, work):
        ax, ay, aw, ah = anchor
        self.work = work
        self.center = ax+aw/2-SIZE/2, ay+ah/2-SIZE/2
        half = min(math.pi/3, max(0, count-1)*.32)
        self.rx = aw/2+SIZE/2+4
        self.ry = max(ah/2, (count-1)*SIZE/(2*math.sin(half)) if half else 0)
        self.angles = [0 if count == 1 else half-2*half*i/(count-1) for i in range(count)]
        raw = [self.ellipse(angle) for angle in self.angles]
        wx, wy, ww, wh = work
        # Shift the whole final arc at an edge, preserving spacing between slots.
        self.shift = tuple(max(low+4-min(p[axis] for p in raw),
                               min(0, low+length-SIZE-4-max(p[axis] for p in raw)))
                           for axis, low, length in [(0, wx, ww), (1, wy, wh)]) if raw else (0, 0)
        self.targets = [self.position(i, 1) for i in range(count)]

    def ellipse(self, angle):
        return self.center[0]-self.rx*math.cos(angle), self.center[1]+self.ry*math.sin(angle)

    def position(self, index, progress):
        ease = 1-(1-progress)**3
        angle = 1.38+(self.angles[index]-1.38)*ease
        x, y = self.ellipse(angle)
        wx, wy, ww, wh = self.work
        return (round(max(wx+4, min(x+self.shift[0]*ease, wx+ww-SIZE-4))),
                round(max(wy+4, min(y+self.shift[1]*ease, wy+wh-SIZE-4))))


class Entry:
    def __init__(self, key, title, activate, interaction=None, icon=None):
        self.key = key
        self.position = None
        self.progress = 0.0
        self.delay = 0.0
        self.opacity = 0.0
        self.hovered = self.pressed = False
        self.on_interaction = interaction or (lambda *_: None)
        self.window = Gtk.Window(type=Gtk.WindowType.POPUP)
        self.window.set_title('Mochi · '+title+'入口')
        self.window.set_wmclass('mochi-function-menu', 'MochiFunctionMenu')
        self.window.set_decorated(False)
        self.window.set_resizable(False)
        self.window.set_keep_above(True)
        self.window.set_accept_focus(False)
        self.window.set_skip_taskbar_hint(True)
        self.window.set_skip_pager_hint(True)
        visual = self.window.get_screen().get_rgba_visual()
        if visual:
            self.window.set_visual(visual)
        self.window.get_style_context().add_class('mochi-function-menu')
        self.window.get_style_context().add_provider(entry_style(), Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION+1)
        self.window.set_size_request(SIZE, SIZE)
        self.fixed = Gtk.Fixed()
        self.window.add(self.fixed)
        self.button = Gtk.Button()
        self.button.set_size_request(48, 48)
        self.button.set_can_focus(False)
        self.button.set_tooltip_text(title+' · 打开管理')
        self.button.get_accessible().set_name(title+'管理')
        self.button.get_style_context().add_class('mochi-function-button')
        if key in ('translation', 'chatgpt'):
            self.button.get_style_context().add_class('mochi-function-'+key)
        self.button.get_style_context().add_provider(entry_style(), Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION+1)
        if icon:
            image = Gtk.Image.new_from_gicon(Gio.FileIcon.new(Gio.File.new_for_path(icon)), Gtk.IconSize.MENU)
            image.set_pixel_size(28)
        else:
            image = Gtk.Image.new_from_icon_name('application-x-addon-symbolic', Gtk.IconSize.MENU)
            image.set_pixel_size(24)
        self.button.add(image)
        self.button.connect('clicked', lambda *_: activate(key))
        self.button.connect('button-press-event', self.press)
        self.button.connect('button-release-event', self.release)
        self.button.connect('grab-broken-event', self.release)
        self.window.add_events(Gdk.EventMask.ENTER_NOTIFY_MASK | Gdk.EventMask.LEAVE_NOTIFY_MASK)
        self.window.connect('enter-notify-event', self.enter)
        self.window.connect('leave-notify-event', self.leave)
        self.fixed.put(self.button, 12, 12)
        self.window.realize()
        native = self.window.get_window()
        # The button stays fixed inside its small popup. Pause the popup's motion
        # on hover/press, so press and release always address the same target.
        native.set_cursor(Gdk.Cursor.new_from_name(native.get_display(), 'pointer'))
        self.draw(0)

    @property
    def interacting(self):
        return self.hovered or self.pressed

    def enter(self, _widget, event):
        if event.detail != Gdk.NotifyType.INFERIOR:
            self.hovered = True
            self.on_interaction(self)
        return False

    def leave(self, _widget, event):
        if event.detail != Gdk.NotifyType.INFERIOR:
            self.hovered = False
            self.on_interaction(self)
        return False

    def press(self, _widget, event):
        if event.button == 1:
            self.pressed = True
            self.on_interaction(self)
        return False

    def release(self, *_):
        self.pressed = False
        self.on_interaction(self)
        return False

    def move(self, position):
        if self.position != position:
            self.position = position
            self.window.move(*position)

    def draw(self, progress, opening=False):
        self.progress = progress
        alpha = 1-(1-min(1, progress/.55))**3
        self.opacity = max(self.opacity, alpha, 1.0 if self.interacting else 0.0) if opening else min(self.opacity, alpha)
        self.button.set_opacity(self.opacity)


class FunctionMenu:
    def __init__(self, display, activate, entries=None):
        self.display = display
        # Registry: later functions add entries here without changing placement.
        self.entries = [Entry(item[0], item[1], self.activate, self.interaction, item[2] if len(item) > 2 else None)
                        for item in (entries if entries is not None else [('translation', '翻译')])]
        self.enabled = bool(self.entries)
        self.on_activate = activate
        self.anchor = None
        self.pointer = None
        self.blocked = True
        self.opened = False
        self.level = 0.0
        self.leave_source = None
        self.animation = None
        self.last_tick = None
        self.layout = None
        self.layout_key = None
        self.resetting = False
        self.settings = Gtk.Settings.get_default()
        self.settings_handler = self.settings.connect('notify::gtk-enable-animations', self.preference)

    def interaction(self, entry):
        if self.resetting or not self.enabled or self.blocked or not entry.window.get_visible():
            return
        if entry.interacting:
            self.cancel_leave()
            self.reveal(True)
            entry.draw(entry.progress, True)
        elif self.layout and entry.progress == 1:
            entry.move(self.layout.targets[self.entries.index(entry)])
        self.animate()

    def preference(self, *_):
        if not self.settings.get_property('gtk-enable-animations'):
            if self.opened:
                self.finish_open()
            else:
                self.hide_now()

    def stop_animation(self):
        if self.animation is not None:
            self.entries[0].window.remove_tick_callback(self.animation)
            self.animation = None

    def finish_open(self):
        self.stop_animation()
        for i, entry in enumerate(self.entries):
            entry.delay = 0
            if self.layout and not entry.interacting:
                entry.move(self.layout.targets[i])
            entry.draw(1, True)
            entry.window.show_all()
        self.level = 1

    def set_enabled(self, enabled):
        self.enabled = bool(enabled) and bool(self.entries)
        if not self.enabled:
            self.cancel_leave()
            self.hide_now()

    def left_edge(self, anchor, work):
        """Reserve the icon arc so a left-side bubble cannot cover its input."""
        if not self.enabled or not self.entries:return None
        key = anchor, len(self.entries), work
        layout = self.layout if self.layout_key == key else ArcLayout(*key)
        return min(x for x, _ in layout.targets)

    def snapshot(self, data):
        scale = self.display.get_default_screen().get_root_window().get_scale_factor()
        self.anchor = tuple(v/scale for v in data['anchor']) if data['anchor'] else None
        self.pointer = tuple(v/scale for v in data['pointer']) if data['pointer'] else None
        self.blocked = data['blocked']
        if not self.enabled or self.blocked or self.anchor is None or self.pointer is None:
            self.cancel_leave()
            self.hide_now()
            return
        ax, ay, aw, ah = self.anchor
        monitor = self.display.get_monitor_at_point(round(ax+aw/2), round(ay+ah/2))
        work = monitor.get_workarea()
        layout_key = self.anchor, len(self.entries), (work.x, work.y, work.width, work.height)
        if self.layout_key != layout_key:
            self.layout_key = layout_key
            self.layout = ArcLayout(*layout_key)
            for i, entry in enumerate(self.entries):
                if not entry.interacting:
                    entry.move(self.layout.position(i, entry.progress))
        over_pet = contains(self.anchor, self.pointer, 12)
        # The retention corridor is logical only: it never intercepts desktop clicks.
        over_path = self.opened and any(contains((min(ax, e.position[0]), min(ay, e.position[1]),
                           max(ax+aw, e.position[0]+SIZE)-min(ax, e.position[0]),
                           max(ay+ah, e.position[1]+SIZE)-min(ay, e.position[1])), self.pointer, 10)
                           for e in self.entries)
        if over_pet or over_path or any(e.interacting for e in self.entries):
            self.cancel_leave()
            self.reveal(True)
        elif self.opened and self.leave_source is None:
            self.leave_source = GLib.timeout_add(LEAVE_MS, self.leave)

    def cancel_leave(self):
        if self.leave_source is not None:
            GLib.source_remove(self.leave_source)
            self.leave_source = None

    def leave(self):
        self.leave_source = None
        self.reveal(False)
        return False

    def reveal(self, visible):
        if self.opened == visible:
            return
        self.opened = visible
        if not self.settings.get_property('gtk-enable-animations'):
            self.finish_open() if visible else self.hide_now()
            return
        for i, entry in enumerate(self.entries):
            entry.delay = i*STAGGER_SECONDS if visible and entry.progress == 0 else 0
            if not visible:
                entry.delay = (len(self.entries)-1-i)*STAGGER_SECONDS*.6 if entry.progress else 0
        if visible:
            # The first mapped popup drives GTK's frame clock; later entries map
            # only when their own entrance starts, leaving empty space clickable.
            for i, entry in enumerate(self.entries):
                if self.layout and entry.progress == 0 and not entry.interacting:
                    entry.move(self.layout.position(i, 0))
            self.entries[0].window.show_all()
        self.animate()

    def animate(self):
        # Later entries travel through the same arc. Stop the group while the
        # user targets one, so another popup cannot pass over its click region.
        if self.opened and any(e.interacting for e in self.entries):
            self.stop_animation()
            return
        pending = any(e.progress != (1 if self.opened else 0) and
                      (not self.opened or not e.interacting) for e in self.entries)
        if pending and self.animation is None:
            # Begin on the first drawable frame. Loading a plugin SVG or GTK
            # CSS can delay that frame; charging the delay to the animation
            # would collapse the intended stagger between entries.
            self.last_tick = None
            self.animation = self.entries[0].window.add_tick_callback(self.tick)

    def tick(self, _widget, _clock):
        if self.opened and any(e.interacting for e in self.entries):
            self.animation = None
            return False
        now = time.monotonic()
        dt = 0 if self.last_tick is None else now-self.last_tick
        self.last_tick = now
        for i, entry in enumerate(self.entries):
            if self.opened and entry.interacting:
                continue
            remaining = max(0, dt-entry.delay)
            entry.delay = max(0, entry.delay-dt)
            progress = max(0, min(1, entry.progress + remaining/(ENTER_SECONDS if self.opened else -EXIT_SECONDS)))
            if self.layout:
                entry.move(self.layout.position(i, progress))
            entry.draw(progress, self.opened)
            if progress > 0 and not entry.window.get_visible():
                entry.window.show_all()
            elif progress == 0 and i != 0:
                entry.window.hide()
        self.level = min(e.progress for e in self.entries)
        finished = all(e.progress == (1 if self.opened else 0) or self.opened and e.interacting for e in self.entries)
        if finished:
            self.animation = None
            if not self.opened:
                for e in self.entries:
                    e.window.hide()
        return not finished

    def hide_now(self):
        self.resetting = True
        self.cancel_leave()
        self.stop_animation()
        self.opened = False
        self.level = 0
        for e in self.entries:
            e.hovered = e.pressed = False
            e.delay = 0
            e.draw(0)
            e.window.hide()
        self.resetting = False

    def destroy(self):
        self.hide_now()
        self.settings.disconnect(self.settings_handler)
        for entry in self.entries:
            entry.window.destroy()

    def activate(self, key):
        self.cancel_leave()
        self.hide_now()
        self.on_activate(key)

    def inspect(self):
        return {'open': self.opened, 'progress': self.level, 'animating': self.animation is not None,
                'leaving': self.leave_source is not None, 'anchor': self.anchor,
                'pointer': self.pointer, 'blocked': self.blocked,
                'entries': [{'id': e.key, 'position': e.position, 'size': SIZE,
                             'progress': e.progress, 'opacity': e.opacity, 'held': e.interacting,
                             'target': self.layout.targets[i] if self.layout else None,
                             'visible': e.window.get_visible(), 'window': e.window.get_window().get_xid()}
                            for i, e in enumerate(self.entries)]}
