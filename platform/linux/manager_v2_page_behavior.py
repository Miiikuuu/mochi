"""Behavior page: owns only its presentation controls."""
from manager_v2_common import Gtk, Page, styled, text
from manager_i18n import _


class BehaviorPage(Page):
    def __init__(self, *args):
        super().__init__(*args)
        self.behavior_controls = {}
        self.behavior_sliders = {}
        self.behavior_status = {}
        self._behavior(self.state.draft.value)

    def _behavior(self, theme):
        self.workspace_content.append(text('Behavior', 'section-title'))
        for field, title, minimum, maximum, suffix in (
                ('size', 'Character size', 128, 512, 'px'),
                ('idle_after', 'Idle timeout', 0, 3600, 's')):
            group = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['sm']), 'behavior-field')
            group.append(text(title, 'field-label'))
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['sm'])
            slider = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, minimum, maximum, 1)
            slider.set_hexpand(True)
            slider.set_draw_value(False)
            slider.set_value(theme[field])
            slider.update_property([Gtk.AccessibleProperty.LABEL], [_('%s slider') % _(title)])
            spinner = Gtk.SpinButton.new_with_range(minimum, maximum, 1)
            spinner.set_value(theme[field])
            spinner.set_numeric(True)
            spinner.update_property([Gtk.AccessibleProperty.LABEL], [_(title)])
            slider.connect('value-changed', lambda widget, identity=field:
                           self._behavior_changed(identity, round(widget.get_value())))
            spinner.connect('value-changed', lambda widget, identity=field:
                            self._behavior_changed(identity, widget.get_value_as_int()))
            row.append(slider)
            row.append(spinner)
            status = text('', 'muted')
            row.append(status)
            group.append(row)
            self.workspace_content.append(group)
            self.behavior_controls[field] = spinner
            self.behavior_sliders[field] = slider
            self.behavior_status[field] = status
            self._set_behavior_status(field, theme[field], suffix)
        self.workspace_content.append(text('Saved changes stay in the draft until you apply them.', 'field-help'))


    def _set_behavior_status(self, field, value, suffix):
        self.behavior_status[field].set_text(_('Disabled') if field == 'idle_after' and value == 0 else f'{value} {suffix}')


    def _behavior_changed(self, field, value):
        spinner = self.behavior_controls[field]
        slider = self.behavior_sliders[field]
        if spinner.get_value_as_int() != value:
            spinner.set_value(value)
        if round(slider.get_value()) != value:
            slider.set_value(value)
        self._set_behavior_status(field, value, 'px' if field == 'size' else 's')
        self.callbacks['behavior'](field, value)
