"""Installed Bubble selection and sample preview."""
from manager_v2_common import Gtk, Pango, Page, styled, text
from manager_i18n import _


from manager_v2_bubble_preview import BubblePreview


class BubblePage(Page):
    def __init__(self, *args):
        super().__init__(*args)
        self.bubble_options = {}
        self._bubble(self.state.draft.value)

    def __getattr__(self, name):
        return getattr(self.preview, name)

    def dispose(self):
        self.preview.dispose()
        super().dispose()

    def _bubble(self, theme):
        self.workspace_content.append(text('Bubble', 'section-title'))
        self.workspace_content.append(text('Choose the material used for speech and replies.', 'section-hint'))
        if theme['bubble'] not in self.bubble_choices:
            self.workspace_content.append(text(theme['bubble'], 'muted'))
            self.workspace_content.append(text('This Bubble is missing or pending removal. Choose an installed replacement before Apply.', 'section-hint'))
        choices = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['sm']), 'bubble-choices')
        previous = None
        for key, name in self.bubble_choices.items():
            choice = Gtk.CheckButton()
            label = text(name)
            label.set_ellipsize(Pango.EllipsizeMode.END)
            choice.set_child(label)
            choice.update_property([Gtk.AccessibleProperty.LABEL], [name])
            choice.set_tooltip_text(name)
            choice.set_hexpand(True)
            if previous is not None:
                choice.set_group(previous)
            choice.set_active(key == theme['bubble'])
            choice.connect('toggled', lambda widget, identity=key:
                           self.callbacks['bubble'](identity) if widget.get_active() else None)
            choices.append(choice)
            self.bubble_options[key] = choice
            previous = choice
        self.workspace_content.append(choices)
        self.preview = BubblePreview(self.layout)
        self.bubble_stage = self.preview.root
        self.workspace_content.append(self.preview.root)
        self.workspace_content.append(text('Saved Bubble changes wait for Apply.', 'field-help'))
