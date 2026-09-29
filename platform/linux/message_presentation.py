"""Choose a conversation or a one-shot speech view without changing producers."""
from conversation_history import ConversationHistory
from conversation_view import ConversationView, materials
from speech_bubble import SpeechBubble


class MessagePresentation:
    def __init__(self, assets, close, action, resized, reply=None, history=None):
        self.assets = assets
        materials(assets)
        self.history = history if history is not None else ConversationHistory()
        self.arguments = (assets, close, action, resized)
        self.reply = reply
        self.single = SpeechBubble(*self.arguments, reply=reply)
        self.conversation = None
        self.active = self.single

    @property
    def window(self):return self.active.window

    @property
    def position(self):return self.active.position

    def render(self, message):
        if message.content.reply:
            self.history.add(message)
            if self.conversation is None:
                self.conversation = ConversationView(*self.arguments, self.reply, self.history)
            view = self.conversation
        else:view = self.single
        if view is not self.active:
            visible = self.active.window.get_visible()
            self.active.hide()
            self.active = view
            view.render(message)
            if visible:view.show()
        else:view.render(message)

    def show(self):self.active.show()
    def hide(self):self.active.hide()
    def clear(self):self.active.clear()
    def follow(self, anchor, work, left_limit=None):self.active.follow(anchor, work, left_limit)
    def inspect(self):return self.active.inspect()

    def forget_source(self, source):
        if self.history.channel and self.history.channel[0] == source:
            self.history.clear()
            if self.conversation:
                if self.active is self.conversation:self.active = self.single
                self.conversation.destroy()
                self.conversation = None

    def adopt_draft(self, previous):
        """Changing a skin does not change the in-flight input/ACK revision."""
        old = previous.conversation
        if not old or not old.message:return
        self.render(old.message)
        target, source = self.conversation.composer, old.composer
        target.entry.set_text(source.entry.get_text())
        for name in ('key', 'clear_id', 'spec', 'submitted', 'revision'):
            setattr(target, name, getattr(source, name))
        target.error.set_text(source.error.get_text())
        target.error.set_visible(source.error.get_visible())
        target.sensitivity()

    def destroy(self):
        self.single.destroy()
        if self.conversation:self.conversation.destroy()
