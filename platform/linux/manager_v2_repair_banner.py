"""Independent resource repair presentation, separate from revision conflicts."""
from manager_v2_common import Gtk, Pango, styled, text, clear
from manager_i18n import _


class RepairBanner:
    def __init__(self, layout, callbacks):
        self.callbacks = callbacks
        self.entries = ()
        self.root = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=layout['spacing']['sm']), 'config-banner')
        self.root.set_visible(False)
        self.root.append(text('Resources need repair', 'config-banner-text'))
        self.root.append(text('Locate, replace or remove unavailable resources before Apply.', 'field-help'))
        self.details = Gtk.Expander(label=_('Details'))
        self.details.set_focusable(True)
        self.listing = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=layout['spacing']['xs'])
        self.details.set_child(self.listing)
        self.root.append(self.details)
        self.actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=layout['spacing']['sm'])
        self.root.append(self.actions)
        self.buttons = {}

    def update(self, entries):
        entries = tuple((item['kind'], item['technical_id'], item['label']) for item in entries)
        if entries == self.entries:
            return
        self.entries = entries
        clear(self.listing)
        clear(self.actions)
        self.buttons.clear()
        self.root.set_visible(bool(entries))
        for _kind, identity, reason in entries:
            label = text(_(reason)+': '+identity, 'field-help')
            label.set_wrap_mode(Pango.WrapMode.WORD_CHAR)
            self.listing.append(label)
        for kind, title in (('characters', 'Repair characters'), ('bubbles', 'Repair Bubble'), ('plugins', 'Repair features')):
            if any(entry[0] == kind for entry in entries):
                button = Gtk.Button(label=_(title))
                button.connect('clicked', lambda _button, value=kind: self.callbacks['repair'](value))
                button.update_property([Gtk.AccessibleProperty.LABEL], [_(title)])
                self.actions.append(button)
                self.buttons[kind] = button

    def responsive(self, compact):
        self.actions.set_orientation(Gtk.Orientation.VERTICAL if compact else Gtk.Orientation.HORIZONTAL)
