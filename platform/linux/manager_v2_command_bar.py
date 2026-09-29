"""CommandBar shell component."""
from manager_v2_common import Gtk, styled, text
from manager_i18n import _


class CommandBar:
    def __init__(self, layout, callbacks):
        self.layout, self.callbacks = layout, callbacks
        gap = layout['spacing']
        self.command_bar = styled(Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=gap['lg']), 'command-bar')
        self.command_text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=gap['xs'])
        self.command_text.set_hexpand(True)
        self.runtime_status = text('Stopped', 'runtime-status')
        self.pending_status = text('', 'pending')
        self.pending_status.set_ellipsize(3)
        self.command_text.append(text('Theme runtime', 'field-label'))
        self.command_text.append(self.runtime_status)
        self.command_text.append(self.pending_status)
        self.command_bar.append(self.command_text)
        self.apply_button = styled(Gtk.Button(label=_('Start theme')), 'apply-button')
        self.apply_button.connect('clicked', lambda *_: self.callbacks['apply']())
        self.command_bar.append(self.apply_button)
        self.root = self.command_bar

    def update_runtime(self, info=None):
        if info is None:
            return
        collapsed = info.get('collapsed', False)
        self.apply_button.set_visible(not collapsed)
        self.pending_status.set_visible(not collapsed)
        self.command_bar.add_css_class('collapsed') if collapsed else self.command_bar.remove_css_class('collapsed')
        self.runtime_status.set_text(_(info['status']))
        self.pending_status.set_text(_(info['pending']))
        self.pending_status.set_tooltip_text(_(info['pending']))
        self.apply_button.set_label(_(info['action']))
        self.apply_button.set_sensitive(not info['busy'] and bool(info['action']))


    def responsive(self, compact):
        self.command_bar.set_orientation(Gtk.Orientation.VERTICAL if compact else Gtk.Orientation.HORIZONTAL)
