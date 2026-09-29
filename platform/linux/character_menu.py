"""A small, explicit context menu for a desktop character."""
import subprocess
from manager_launcher import manager_command
from pathlib import Path

import gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gdk, GLib, Gtk

from mochi_paths import DATA as ROOT


class CharacterMenu:
    def __init__(self, remove):
        self.remove = remove
        self.target = None
        self.menu = Gtk.Menu()
        self.menu.get_style_context().add_class('mochi-character-context')
        style = Gtk.CssProvider()
        style.load_from_path(str(ROOT/'assets/icons/character-context.css'))
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), style, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION+1)
        self.manage = Gtk.MenuItem.new_with_label('打开 Mochi 管理')
        self.manage.connect('activate', self.open_manager)
        self.menu.append(self.manage)
        self.menu.append(Gtk.SeparatorMenuItem())
        self.remove_item = Gtk.MenuItem.new_with_label('移除这个角色')
        self.remove_item.get_style_context().add_class('mochi-character-remove')
        self.remove_item.connect('activate', self.remove_character)
        self.menu.append(self.remove_item)

    def show(self, target, x, y, time):
        self.target = target
        self.menu.popdown()
        self.menu.show_all()
        root = Gdk.get_default_root_window()
        scale = root.get_scale_factor()
        rect = Gdk.Rectangle()
        rect.x, rect.y, rect.width, rect.height = round(x/scale), round(y/scale), 1, 1
        event = Gdk.Event.new(Gdk.EventType.BUTTON_RELEASE)
        event.window, event.button, event.time = root, 3, time
        event.x_root, event.y_root = x/scale, y/scale
        event.x, event.y = rect.x, rect.y
        pointer = Gdk.Display.get_default().get_default_seat().get_pointer()
        event.set_device(pointer)
        event.set_source_device(pointer)
        self.menu.popup_at_rect(root, rect, Gdk.Gravity.SOUTH_WEST, Gdk.Gravity.NORTH_WEST, event)
        GLib.timeout_add(70, self.report_popup)

    def report_popup(self):
        window = self.menu.get_window()
        location = window.get_origin()[1:] if window else None
        print(f'Character menu target={self.target} visible={self.menu.get_visible()} mapped={self.menu.get_mapped()} at={location}', flush=True)
        return False

    def hide(self):
        self.menu.popdown()

    def open_manager(self, *_):
        subprocess.Popen(manager_command(), cwd=ROOT, start_new_session=True)

    def remove_character(self, *_):
        if self.target is not None:
            self.remove(self.target)

    def inspect(self):
        visible = self.menu.get_visible()
        items = {}
        if visible and self.menu.get_window():
            valid, x, y = self.menu.get_window().get_origin()
            if valid:
                for name, item in [('manage', self.manage), ('remove', self.remove_item)]:
                    box = item.get_allocation()
                    items[name] = [x+box.x, y+box.y, box.width, box.height]
        return {'visible': visible, 'target': self.target if visible else None, 'items': items}

    def destroy(self):
        self.menu.destroy()
