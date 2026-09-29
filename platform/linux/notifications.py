"""A second, non-translation producer using exactly the same message interface."""
from dataclasses import replace
from messages import Action, Content, MessageState


class Notifications:
    source_id = 'notification'

    def __init__(self, messages, clipboard):
        self.messages, self.clipboard = messages, clipboard
        messages.register(self.source_id, self.action)

    def publish(self, text, title='通知', ttl_ms=8000, priority=10):
        if not isinstance(text, str) or not text.strip():
            raise ValueError('通知内容不能为空。')
        content = Content(title, text, actions=(Action('copy', '复制内容'), Action('dismiss', '知道了', align='end')), kind='notice')
        return self.messages.publish(self.source_id, content, priority=priority, ttl_ms=ttl_ms)

    def action(self, message_id, action_id):
        message = self.messages.get(message_id)
        if not message or message.source != self.source_id:
            return
        if action_id == 'dismiss':
            self.messages.dismiss(message_id)
        elif action_id == 'copy':
            self.clipboard(message.content.text)
            content = replace(message.content, actions=tuple(replace(a, label='已复制') if a.id == 'copy' else a for a in message.content.actions))
            self.messages.update(self.source_id, message_id, content=content, state=MessageState.INFO)
