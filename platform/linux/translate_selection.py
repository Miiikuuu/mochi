#!/usr/bin/python3
"""One-shot global shortcut: read PRIMARY and hand it to the desktop plugin.

No keyboard injection, CLIPBOARD reads/writes, window activation or daemon.
Use XWayland's selection bridge for both X11 and native Wayland applications.
"""
import fcntl
import json
import os
import socket
import stat
import struct
import sys

# The desktop host is X11/XWayland even in a Wayland session. PRIMARY is
# bridged by the compositor; a background native Wayland client lacks focus.
os.environ['GDK_BACKEND'] = 'x11'
import gi
gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
from gi.repository import Gdk, Gio, GLib, Gtk
from desktop_ipc import socket_path

MAX_TEXT = 12000
READ_TIMEOUT_MS = 1500


class SelectionError(Exception):
    pass


def selected_text():
    if not Gtk.init_check()[0]:
        raise SelectionError('无法连接桌面显示，请在图形桌面会话中使用。')
    selection = Gtk.Clipboard.get(Gdk.SELECTION_PRIMARY)
    loop = GLib.MainLoop()
    result = {'done': False, 'text': None}

    def received(_clipboard, text, _data):
        result.update(done=True, text=text)
        if loop.is_running():
            loop.quit()

    def timeout():
        loop.quit()
        return False

    timer = GLib.timeout_add(READ_TIMEOUT_MS, timeout)
    try:
        selection.request_text(received, None)
        # Some selection providers invoke the callback synchronously.
        if not result['done']:
            loop.run()
    finally:
        if result['done']:
            GLib.source_remove(timer)
    if not result['done']:
        raise SelectionError('读取选中文字超时，请重新选择后重试。')
    text = result['text']
    if not text or not text.strip():
        raise SelectionError('未读取到选中文字。请先选中文字；若软件不支持选区读取，可复制后粘贴到桌宠翻译面板。')
    if len(text.encode('utf-16-le')) // 2 > MAX_TEXT:
        raise SelectionError('选中文字超过 12000 个字符，请缩小选区后重试。')
    return text


def notify(message):
    # No source text is logged or included in notifications.
    print(message, file=sys.stderr)
    if os.environ.get('MOCHI_SHORTCUT_TEST') == '1':
        return
    try:
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        bus.call_sync('org.freedesktop.Notifications', '/org/freedesktop/Notifications',
                      'org.freedesktop.Notifications', 'Notify',
                      GLib.Variant('(susssasa{sv}i)', ('Mochi', 0, 'preferences-desktop-locale',
                                   '桌宠翻译', message, [], {}, 5000)),
                      None, Gio.DBusCallFlags.NONE, 1500, None)
    except GLib.Error:
        pass


def translate_selection():
    path = socket_path()
    try:
        info = path.lstat()
        parent = path.parent.lstat()
        if (not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid()
                or not stat.S_ISDIR(parent.st_mode) or parent.st_uid != os.getuid()
                or parent.st_mode & 0o077):
            raise SelectionError('桌宠连接无效，请重新启动 Mochi。')
        # Simultaneous/repeated key activations must not start duplicate jobs.
        fd = os.open(path.with_suffix('.shortcut.lock'), os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, 'r+') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(2.5)
                client.connect(str(path))
                uid = struct.unpack('3i', client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[1]
                if uid != os.getuid():
                    raise SelectionError('桌宠连接不属于当前用户。')
                text = selected_text()
                frame = json.dumps({'type': 'translate', 'text': text}, ensure_ascii=False).encode() + b'\n'
                client.sendall(frame)
                response = bytearray()
                while b'\n' not in response:
                    chunk = client.recv(8192)
                    if not chunk or len(response) + len(chunk) > 8192:
                        raise SelectionError('桌宠没有确认接收，请重试。')
                    response.extend(chunk)
                try:
                    data = json.loads(response.split(b'\n', 1)[0])
                except (ValueError, UnicodeError):
                    raise SelectionError('桌宠返回了无效响应。') from None
                if not isinstance(data, dict) or data.get('accepted') is not True:
                    raise SelectionError('桌宠未能接收翻译，请在翻译面板中重试。')
            # GNOME can repeat a held accelerator. Debounce only the trigger;
            # model execution belongs to the desktop and is fully asynchronous.
            import time
            time.sleep(0.6)
    except (FileNotFoundError, ConnectionRefusedError):
        raise SelectionError('请先启动 Mochi 桌宠，再使用快捷键翻译。') from None
    except (TimeoutError, ConnectionResetError, BrokenPipeError):
        raise SelectionError('桌宠暂时没有响应，请重试。') from None


def main():
    try:
        translate_selection()
        return 0
    except SelectionError as error:
        notify(str(error))
    except OSError:
        notify('无法读取选区或连接桌宠，请重新选择文字后重试。')
    return 1


if __name__ == '__main__':
    sys.exit(main())
