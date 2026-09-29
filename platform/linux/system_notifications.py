"""Optional desktop toasts for messages already accepted by Mochi."""
import os
import shutil
import subprocess


class SystemNotifications:
    def __init__(self, *, test=False):
        self.command = shutil.which('notify-send')
        self.history = [] if test else None

    def publish(self, plugin):
        if plugin == 'codex-notify':
            title, body = 'Mochi · Codex', '指定任务已结束，打开桌宠查看。'
        elif plugin == 'chatgpt':
            title, body = 'Mochi · ChatGPT', '收到新回复，打开桌宠查看。'
        else:
            title, body = 'Mochi', '收到新消息，打开桌宠查看。'
        if self.history is not None:
            self.history.append({'plugin': plugin, 'title': title, 'body': body})
            del self.history[:-32]
            return True
        if not self.command:
            return False
        try:
            subprocess.Popen([self.command, '--app-name=Mochi', '--expire-time=8000',
                              '--icon=dialog-information', '--', title, body],
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL, close_fds=True)
        except OSError:
            return False
        return True
