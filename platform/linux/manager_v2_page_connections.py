"""Connections status center. Values only; commands go to the presenter."""
from manager_v2_common import Page, Gtk, text, styled
from manager_i18n import _, relative_time
from manager_connections import project_connections


class ConnectionsPage(Page):
    def __init__(self, *args):
        super().__init__(*args)
        self.root.set_spacing(self.layout['spacing']['lg'])
        self.root.append(text('Connections', 'page-title'))
        self.grid = Gtk.Grid(column_spacing=self.layout['spacing']['lg'], row_spacing=self.layout['spacing']['lg'], column_homogeneous=True)
        self.root.append(self.grid)
        self.cards = []
        self.columns = None
        self.labels = {}
        self.buttons = {}
        for title, fields in (
                ('ChatGPT', ('connection', 'conversation', 'heartbeat', 'reply')),
                ('Translation', ('translation', 'current_model', 'fast', 'catalog')),
                ('Codex notifications', ('notifications', 'event', 'health'))):
            card = styled(Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=self.layout['spacing']['sm']), 'feature-card')
            card.append(text(title, 'section-title'))
            for field in fields:
                label = text('', 'section-hint')
                label.set_selectable(True)
                card.append(label)
                self.labels[field] = label
            actions = [('Disconnect', 'connection_disconnect'), ('Diagnostics', 'diagnostics')] if title == 'ChatGPT' else [('Refresh models', 'models_refresh')] if title == 'Translation' else []
            for name, callback in actions:
                button = Gtk.Button(label=_(name))
                button.update_property([Gtk.AccessibleProperty.LABEL], [_(name)])
                button.connect('clicked', lambda _button, key=callback: self.callbacks[key]())
                card.append(button)
                self.buttons[callback] = button
            if title == 'Translation':
                expander = Gtk.Expander(label=_('Available models'))
                expander.set_focusable(True)
                self.model_list = text('', 'section-hint')
                self.model_list.set_selectable(True)
                expander.set_child(self.model_list)
                card.append(expander)
            self.cards.append(card)
        self.set_width(420)
        self.update_connections()

    def update_connections(self):
        model = project_connections(self.state.runtime, self.state.models, self.state.session_connection)
        catalog = self.state.models
        reply = ('Waiting for Send follow-up confirmation' if model.confirmation_pending else
                 {'idle': 'No pending reply', 'queued': 'Reply queued', 'delivering': 'Reply delivering',
                  'sent': 'Reply sent', 'uncertain': 'Delivery uncertain', 'failed': 'Reply failed'}[model.reply])
        values = {'connection': _('Connected' if model.connected else 'Disconnected'),
                  'conversation': _('Conversation')+': '+(model.conversation or '—'),
                  'heartbeat': _('Last heartbeat')+': '+(relative_time(seconds=model.heartbeat_age) if model.heartbeat_age is not None else '—'),
                  'reply': _(reply), 'translation': _('Runtime available' if model.translation_available else 'Runtime unavailable'),
                  'current_model': _('Current model')+': '+(model.current_model or '—'),
                  'fast': _('Fast supported' if model.fast_tier else 'Fast capability unknown' if model.current_model not in catalog.models else 'Fast unavailable'),
                  'catalog': (_('Loading…') if catalog.loading else _('Model catalog')+': '+str(len(catalog.models)))+'\n'+_('Last synced')+': '+relative_time(catalog.last_synced)+ ('\n'+_('Model refresh failed') if catalog.error else ''),
                  'notifications': _(model.notifications), 'event': _('Last event')+': '+(model.last_event or '—'),
                  'health': _('Plugin health')+': '+_(model.health)}
        for key, value in values.items():
            self.labels[key].set_text(value)
        self.labels['heartbeat'].set_tooltip_text(model.heartbeat_at or None)
        self.labels['catalog'].set_tooltip_text(catalog.last_synced)
        self.model_list.set_text('\n'.join(key+(' · Fast' if value.get('fast_tier') else '') for key, value in catalog.models.items()) or _('Unavailable'))
        self.buttons['models_refresh'].set_sensitive(not catalog.loading)
        self.buttons['connection_disconnect'].set_sensitive(bool(model.conversation) and not self.state.commands.busy)

    def set_width(self, width):
        columns = 2 if width >= self.layout['connectionsTwoColumnWidth'] else 1
        if columns == self.columns:
            return
        self.columns = columns
        for index, card in enumerate(self.cards):
            if card.get_parent():
                self.grid.remove(card)
            self.grid.attach(card, index % columns, index // columns, 1, 1)
