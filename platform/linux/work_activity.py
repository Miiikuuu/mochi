"""Map source-owned task messages to one primary-character work signal."""
from messages import MessageState


def active(messages):
    return any(message.state == MessageState.RUNNING or
               (message.content.reply is not None and message.content.reply.waiting)
               for message in messages)
