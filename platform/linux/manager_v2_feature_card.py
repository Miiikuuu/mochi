"""Feature summary and enable control, without service dependencies."""
from manager_v2_common import Gtk, Pango, styled, text
from manager_i18n import _


class FeatureCard:
    def __init__(self, model, layout, callbacks):
        self.layout, self.callbacks = layout, callbacks
        card = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['sm']), 'feature-card')
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=self.layout['spacing']['md'])
        name = text(model.display_name, 'theme-row-name')
        name.set_hexpand(True)
        name.set_ellipsize(Pango.EllipsizeMode.END)
        row.append(name)
        if model.lifecycle != 'active':
            card.append(text('Pending removal' if model.lifecycle == 'pending_removal' else 'Missing', 'feature-meta'))
        toggle = Gtk.Switch(active=model.enabled)
        toggle.set_sensitive(model.enabled or model.available and model.lifecycle == 'active')
        toggle.update_property([Gtk.AccessibleProperty.LABEL], [model.display_name])
        toggle.connect('notify::active', lambda widget, _param, identity=model.key:
                       self.callbacks['feature'](identity, widget.get_active()))
        row.append(toggle)
        card.append(row)
        self.toggle = toggle
        if not model.available:
            card.append(text('Missing', 'feature-meta'))
            if model.enabled:
                remove = Gtk.Button(label=_('Remove from Theme'))
                remove.connect('clicked', lambda *_: self.callbacks['feature'](model.key, False))
                card.append(remove)
        if model.third_party:
            metadata = 'v' + model.version if model.version else _('Unavailable')
            if model.capabilities:
                metadata += ' · ' + ', '.join(model.capabilities)
            detail = text(metadata, 'feature-meta')
            detail.set_wrap(True)
            detail.set_max_width_chars(28)
            card.append(detail)
            trust = text(' · '.join(_(value) for value in (model.origin, model.review, model.isolation)), 'feature-meta')
            trust.set_wrap(True)
            trust.set_max_width_chars(28)
            card.append(trust)
            isolation = text(model.session_isolation, 'feature-meta')
            isolation.set_wrap(True)
            card.append(isolation)
            if model.isolation == 'unsandboxed':
                warning = text('Runs with your user permissions.', 'feature-meta')
                warning.set_wrap(True)
                card.append(warning)
        elif model.description:
            description = text(model.description, 'section-hint')
            description.set_wrap(True)
            card.append(description)
        self.root = card
