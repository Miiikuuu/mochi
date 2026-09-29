#!/usr/bin/env python3
"""Session-owned message presentation and IPC, composed with independent producers."""
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parent))
from dataclasses import replace
import json
import math
import os
from pathlib import Path
import signal
import socket
import stat
import struct
import sys

import gi
gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
gi.require_version('GdkX11', '3.0')
from gi.repository import Gdk, GdkX11, GLib, Gtk
from bubble_layout import place
from bubble_themes import BubbleThemes
from character_menu import CharacterMenu
from desktop_ipc import socket_path
from function_menu import FunctionMenu
from messages import Content, MessageManager, MessageState
from work_activity import active as work_active
from notifications import Notifications
from system_notifications import SystemNotifications
from message_presentation import MessagePresentation as SpeechBubble
from plugin_runtime import PluginRuntime
from theme_store import ThemeStore

from mochi_paths import DATA as ROOT

class BubbleHost:
    def __init__(self, roles):
        if not roles:
            raise RuntimeError('No primary character')
        self.roles = roles
        self.theme_config = json.loads(os.environ.get('MOCHI_THEME_CONFIG', 'null'))
        self.plugins = self.theme_config.get('plugins', ['translation']) if self.theme_config else ['translation']
        self.primary_id = str(roles[0][0])
        self.primary_name = roles[0][1].split(' · ', 1)[0]
        self.display = Gdk.Display.get_default()
        if not isinstance(self.display, GdkX11.X11Display):
            raise RuntimeError('Mochi messages require the character X11 display')
        self.primary_window = GdkX11.X11Window.foreign_new_for_display(self.display, int(self.primary_id))
        if not self.primary_window:
            raise RuntimeError('No live primary character window')
        self.clients = {}
        self.follow_source = self.expiry_source = None
        self.current_position = self.anchor_rect = None
        self.presentation = 'panel'
        self.shutting_down = False
        self.panel = self.speech = None
        self.panel_owner = None
        self.plugin_runtime = None
        self.parent_buffer = bytearray()
        self.messages = MessageManager(self.primary_id)
        self.work_active = False
        self.test_work_id = None
        self.store = ThemeStore()
        if self.theme_config:
            self.theme_config = self.store.validate(self.theme_config)
        self.notifications = Notifications(self.messages, self.copy_text)
        self.system_notifications = SystemNotifications(test=os.environ.get('MOCHI_BUBBLE_TEST') == '1')
        self.themes = BubbleThemes()
        if self.theme_config:
            self.themes.resolve(self.theme_config['bubble'])
            self.themes.selected = self.theme_config['bubble']
        try:
            self.speech = SpeechBubble(self.themes.resolve(self.themes.selected), self.close_message, self.messages.dispatch, self.follow_once, reply=self.messages.submit_reply)
        except (OSError, ValueError, GLib.Error) as error:
            print(f'Using default bubble after material error: {error}', flush=True)
            self.themes.selected = 'default'
            self.speech = SpeechBubble(self.themes.resolve('default'), self.close_message, self.messages.dispatch, self.follow_once, reply=self.messages.submit_reply)
        self.menu = FunctionMenu(self.display, self.open_function, entries=[])
        self.context_menu = CharacterMenu(self.send_remove)
        self.plugin_runtime = PluginRuntime(self, self.store.plugins)
        self.plugin_runtime.apply(self.theme_config, self.plugins)
        self.rebuild_menu()
        self.messages.subscribe(self.messages_changed)
        self.sock_path = socket_path()
        self.sock_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        parent = self.sock_path.parent.lstat()
        if not stat.S_ISDIR(parent.st_mode) or parent.st_uid != os.getuid() or parent.st_mode & 0o077:
            raise RuntimeError('Bubble socket directory must be private to the current user')
        if self.sock_path.exists() or self.sock_path.is_symlink():
            info = self.sock_path.lstat()
            if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid():
                raise RuntimeError('Refusing to replace an unrelated socket path')
            with socket.socket(socket.AF_UNIX) as probe:
                try:
                    probe.connect(str(self.sock_path))
                except (ConnectionRefusedError, FileNotFoundError):
                    self.sock_path.unlink(missing_ok=True)
                else:
                    raise RuntimeError('A bubble host already owns this desktop')
        self.server = socket.socket(socket.AF_UNIX)
        self.server.bind(str(self.sock_path))
        os.chmod(self.sock_path, 0o600)
        self.server.listen(8)
        self.server.setblocking(False)
        GLib.io_add_watch(self.server.fileno(), GLib.IO_IN, self.accept)
        GLib.io_add_watch(sys.stdin.fileno(), GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR, self.parent_input)
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, self.quit)
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT, self.quit)
        print(f'Bubble host ready: {self.sock_path}', flush=True)


    def alive_window(self):
        self.anchor_rect = None
        window = self.primary_window
        if window.is_destroyed() or not window.is_visible():
            return None
        # Trap removal between the visibility check and XTranslateCoordinates.
        self.display.error_trap_push()
        try:
            valid, x, y = window.get_origin()
            width, height = window.get_width(), window.get_height()
            self.display.sync()
        finally:
            error = self.display.error_trap_pop()
        if error or not valid:
            return None
        self.anchor_rect = (x, y, width, height)
        return window


    def accept(self, *_):
        try:
            client, _ = self.server.accept()
        except BlockingIOError:
            return True
        peer, uid, _gid = struct.unpack('3i', client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        if uid != os.getuid() or len(self.clients) >= 8:
            client.close()
            return True
        client.setblocking(False)
        try:
            confined = 'NoNewPrivs:\t1' in Path(f'/proc/{peer}/status').read_text()
        except OSError:
            client.close()
            return True
        state = {'client': client, 'data': bytearray(), 'confined': confined}
        self.clients[client.fileno()] = state
        state['watch'] = GLib.io_add_watch(client.fileno(), GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR,
                                           lambda _fd, _cond: self.read_client(state))
        state['timer'] = GLib.timeout_add(3000, lambda: self.close_client(state))
        return True


    def close_client(self, state):
        self.clients.pop(state['client'].fileno(), None)
        for name in ['watch', 'timer']:
            if state.get(name):
                GLib.source_remove(state.pop(name))
        state['client'].close()
        return False


    def read_client(self, state):
        try:
            chunk = state['client'].recv(65536)
            if not chunk:
                self.close_client(state)
                return False
            state['data'].extend(chunk)
            if len(state['data']) > 65536:
                raise ValueError('选择的文字过长。')
            if b'\n' not in state['data']:
                return True
            message = json.loads(state['data'].split(b'\n', 1)[0])
            if state.get('confined') and (message.get('type') in ('plugin_reply_bridge', 'connection_disconnect', 'inspect') or
                message.get('type') == 'plugin_message' and (message.get('plugin') == 'chatgpt' or message.get('conversation_key'))):
                raise ValueError('Session-sensitive desktop requests require a trusted bridge.')
            response = self.handle(message)
        except BlockingIOError:
            return True
        except (OSError, ValueError, TypeError, AttributeError) as error:
            response = {'error': str(error)}
        try:
            state['client'].sendall(json.dumps(response, ensure_ascii=False).encode() + b'\n')
        except OSError:
            pass
        self.close_client(state)
        return False


    def parent_input(self, fd, condition):
        data = os.read(fd, 65536)
        if condition & (GLib.IO_HUP | GLib.IO_ERR) or not data:
            self.quit()
            return False
        self.parent_buffer.extend(data)
        latest = None
        while b'\n' in self.parent_buffer:
            line, _, rest = self.parent_buffer.partition(b'\n')
            self.parent_buffer = bytearray(rest)
            message = json.loads(line)
            if message.get('type') == 'pointer':
                latest = message
            elif message.get('type') == 'context_menu':
                target = message.get('window')
                x, y, time = message.get('x'), message.get('y'), message.get('time')
                if (type(target) is int and target in [identity for identity, _ in self.roles]
                        and type(x) is int and type(y) is int and abs(x) < 1000000 and abs(y) < 1000000
                        and type(time) is int and 0 <= time <= 0xffffffff):
                    self.display.error_trap_push()
                    try:
                        window = GdkX11.X11Window.foreign_new_for_display(self.display, target)
                        alive = bool(window and not window.is_destroyed() and window.is_visible())
                        self.display.sync()
                    finally:
                        error = self.display.error_trap_pop()
                    if alive and not error:
                        self.menu.hide_now()
                        self.context_menu.show(target, x, y, time)
        if latest is not None:
            if self.context_menu.menu.get_visible():
                self.menu.hide_now()
            else:
                self.menu.snapshot(latest)
            if latest.get('anchor') is None and not self.alive_window():
                self.close_all()
        return True

    def send_remove(self, target):
        frame = json.dumps({'type': 'remove_role', 'window': target}).encode()+b'\n'
        try:
            os.write(sys.stdin.fileno(), frame)
        except OSError as error:
            print(f'Could not remove character: {error}', flush=True)

    def sync_work(self):
        active = work_active(self.messages.messages)
        if active == self.work_active:
            return
        self.work_active = active
        frame = json.dumps({'type': 'work_state', 'window': int(self.primary_id), 'active': active}).encode()+b'\n'
        try:
            os.write(sys.stdin.fileno(), frame)
        except OSError as error:
            print(f'Could not update character work action: {error}', flush=True)

    def copy_text(self, text):
        Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text(text, -1)

    def select_theme(self, name):
        self.require_primary()
        if name == self.themes.selected:return
        try:
            replacement = SpeechBubble(self.themes.resolve(name), self.close_message, self.messages.dispatch, self.follow_once, reply=self.messages.submit_reply, history=self.speech.history)
        except GLib.Error as error:
            raise ValueError('气泡样式无效，请检查素材。') from error
        try:
            self.themes.select(name)
        except OSError:
            replacement.destroy()
            raise
        previous = self.speech
        replacement.adopt_draft(previous)
        self.speech = replacement
        previous.destroy()
        self.messages_changed()
        for plugin in self.plugin_runtime.instances.values():
            plugin.panel.sync_theme()
        if self.theme_config:
            self.theme_config['bubble'] = name

    def require_primary(self):
        if not self.alive_window():
            self.close_all()
            raise ValueError('主角色已关闭，请重新启动主角色后使用插件。')

    def rebuild_menu(self):
        self.menu.destroy()
        self.menu = FunctionMenu(self.display, self.open_function, entries=self.plugin_runtime.entries())

    def open_function(self, key):
        self.require_primary()
        self.plugin_runtime.open(key)

    def show_panel(self, plugin):
        if self.panel and self.panel is not plugin.panel:
            self.panel.window.hide()
        self.panel, self.panel_owner = plugin.panel, plugin.key
        self.show('panel')

    def messages_changed(self):
        if self.shutting_down:
            return
        self.sync_work()
        if self.expiry_source is not None:
            GLib.source_remove(self.expiry_source)
            self.expiry_source = None
        deadline = self.messages.next_deadline
        if deadline is not None:
            delay = max(1, math.ceil((deadline - self.messages.clock()) * 1000))
            self.expiry_source = GLib.timeout_add(delay, self.expire_messages)
        message = self.messages.active
        # A temporary high-priority notice can cover a conversation. Record
        # bound turns as they arrive even while their presentation is covered.
        channel = self.speech.history.channel
        for pending in self.messages.messages:
            reply = pending.content.reply
            if reply and (channel is None or channel == (pending.source, reply.key)):
                title = pending.content.title if pending.content.speaker == 'user' else self.primary_name+' · '+pending.content.title
                title = title.encode('utf-16-le')[:320].decode('utf-16-le', errors='ignore')
                self.speech.history.add(replace(pending, content=replace(pending.content, title=title)))
        if message:
            title = message.content.title if message.content.speaker == 'user' else self.primary_name+' · '+message.content.title
            title = title.encode('utf-16-le')[:320].decode('utf-16-le', errors='ignore')
            self.speech.render(replace(message, content=replace(message.content, title=title)))
            if self.presentation == 'speech':
                self.speech.show()
                self.ensure_following()
        else:
            self.speech.clear()
        self.follow_once()
        if not (self.panel and self.panel.window.get_visible()) and not self.speech.window.get_visible():
            self.stop_following()

    def expire_messages(self):
        self.expiry_source = None
        self.messages.expire()
        if self.expiry_source is None and self.messages.next_deadline is not None:
            self.messages_changed()
        return False

    def show(self, presentation):
        self.require_primary()
        self.presentation = presentation
        if presentation == 'panel':
            self.speech.hide()
            self.panel.show()
        else:
            if self.panel:
                self.panel.window.hide()
            self.messages_changed()
        self.current_position = None
        self.ensure_following()

    def ensure_following(self):
        if self.follow_once() and self.follow_source is None:
            self.follow_source = GLib.timeout_add(50, self.follow_tick)

    def follow_once(self):
        if not self.speech or not ((self.panel and self.panel.window.get_visible()) or self.speech.window.get_visible()):
            return False
        if not self.alive_window():
            self.close_all()
            return False
        x, y, width, height = self.anchor_rect
        monitor = self.display.get_monitor_at_point(x+width//2, y+height//2)
        work = monitor.get_workarea()
        bounds = (work.x, work.y, work.width, work.height)
        if self.presentation == 'speech':
            anchor = self.menu.anchor or self.anchor_rect
            self.speech.follow(anchor, bounds, self.menu.left_edge(anchor, bounds))
            self.current_position = self.speech.position
        else:
            position = place(self.anchor_rect, self.panel.window.get_size(), bounds)
            if position != self.current_position:
                self.panel.window.move(*position)
                self.current_position = position
        return True

    def follow_tick(self):
        keep = self.follow_once()
        if not keep:
            self.follow_source = None
        return keep

    def stop_following(self):
        if self.follow_source is not None:
            GLib.source_remove(self.follow_source)
            self.follow_source = None

    def close_message(self, message_id):
        if message_id:
            self.messages.dismiss(message_id)
        return True

    def close_panel(self):
        if self.panel_owner and self.plugin_runtime:
            self.plugin_runtime.get(self.panel_owner).close_panel()
        self.presentation = 'speech'
        self.messages_changed()
        return True

    def close_all(self):
        previous = self.shutting_down
        self.shutting_down = True
        if self.panel:
            self.panel.window.hide()
        self.speech.clear()
        if self.plugin_runtime:
            self.plugin_runtime.stop()
        self.messages.clear('owner_closed')
        self.stop_following()
        if self.expiry_source is not None:
            GLib.source_remove(self.expiry_source)
            self.expiry_source = None
        self.menu.hide_now()
        self.context_menu.hide()
        self.shutting_down = previous

    def theme_status(self):
        roles = []
        for identity, name in self.roles:
            position, alive = None, False
            self.display.error_trap_push()
            try:
                window = GdkX11.X11Window.foreign_new_for_display(self.display, identity)
                if window and not window.is_destroyed():
                    valid, x, y = window.get_origin()
                    alive = bool(valid and window.is_visible())
                    position = [x, y] if alive else None
                self.display.sync()
            finally:
                if self.display.error_trap_pop():
                    alive, position = False, None
            roles.append({'window': identity, 'name': name, 'alive': alive, 'position': position})
        return {'pid': int(os.environ.get('MOCHI_RENDERER_PID', os.getppid())), 'helper_pid': os.getpid(), 'generation': os.environ.get('MOCHI_RUNTIME_GENERATION'), 'theme': self.theme_config,
                'roles': roles, 'plugins': self.plugins, 'bubble': self.themes.selected,
                'busy': self.plugin_runtime.busy, 'primary': int(self.primary_id),
                'plugin_api': 1, 'plugin_features': ['external_messages', 'external_replies'],
                'plugin_status': self.plugin_runtime.status(), **self.plugin_runtime.extra()}

    def apply_theme(self, theme, translation=None):
        from theme_store import ThemeStore
        store = ThemeStore(bubbles=self.themes)
        theme = store.validate(theme)
        store.require_active_theme(theme)
        if translation is not None:
            from manager_preferences import translation_options
            translation = translation_options(translation)
        # Character identity and geometry require a new desktop session.
        if (self.theme_config or {}).get('actions', {}) != theme.get('actions', {}):
            raise ValueError('动作变更需要重新启动 Theme。')
        if self.theme_config and any(theme[field] != self.theme_config[field]
                                     for field in ('primary', 'characters', 'size', 'idle_after')):
            raise ValueError('角色变更需要重新启动 Theme。')
        if self.theme_config is None:
            args = Path(f'/proc/{int(os.environ.get('MOCHI_RENDERER_PID', os.getppid()))}/cmdline').read_bytes().decode().strip('\0').split('\0')[1:]
            cwd = Path(f'/proc/{int(os.environ.get('MOCHI_RENDERER_PID', os.getppid()))}/cwd').resolve()
            packages = [str((cwd/arg).resolve()) for arg in args if arg.endswith('.mochi')]
            if packages != store.paths(theme)[1]:
                raise ValueError('角色变更需要重新启动 Theme。')
        elif self.theme_config['id'] != theme['id']:
            raise ValueError('切换 Theme 需要重新启动会话。')
        self.select_theme(theme['bubble'])
        self.plugin_runtime.apply(theme, theme['plugins'], {'translation': translation})
        self.plugins = theme['plugins']
        self.theme_config = theme
        self.rebuild_menu()
        if not (self.panel and self.panel.window.get_visible()):
            self.presentation = 'speech'
            self.messages_changed()
        return {'accepted': True}

    def handle(self, request):
        if not isinstance(request, dict):
            raise ValueError('无效的桌宠请求。')
        kind = request.get('type')
        if os.environ.get('MOCHI_BUBBLE_TEST') == '1' and kind == 'test_work':
            active = request.get('active')
            if type(active) is not bool:
                raise ValueError('Invalid test work state')
            if active and self.test_work_id is None:
                self.messages.register('test:work', lambda *_: None)
                self.test_work_id = self.messages.publish('test:work', Content('Test task', 'Working'), MessageState.RUNNING)
            elif active:
                self.messages.update('test:work', self.test_work_id, content=Content('Test task', 'Working'), state=MessageState.RUNNING)
            elif not active and self.test_work_id is not None:
                self.messages.update('test:work', self.test_work_id, content=Content('Test task', 'Done'), state=MessageState.SUCCESS)
            return {'active': self.work_active}
        if kind == 'plugin_open':
            self.open_function(request.get('plugin'))
            return {'accepted': True}
        if kind == 'plugin_message':
            self.require_primary()
            if type(request.get('system_notify', False)) is not bool:
                raise ValueError('系统提醒参数无效。')
            result = self.plugin_runtime.receive_external(request)
            if request.get('system_notify'):
                self.system_notifications.publish(request.get('plugin'))
            return result
        if kind == 'plugin_reply_bridge':
            self.require_primary()
            return self.plugin_runtime.reply_bridge(request)
        if kind == 'connection_disconnect':
            raise ValueError('Session disconnect requires the authenticated Manager protocol.')
        if kind == 'theme_status':
            return self.theme_status()
        if kind == 'theme_apply':
            return self.apply_theme(request.get('theme'), request.get('translation'))
        if kind == 'theme_stop':
            if request.get('pid') != int(os.environ.get('MOCHI_RENDERER_PID', os.getppid())):
                raise ValueError('Theme 已改变，请刷新后重试。')
            parent = int(os.environ.get('MOCHI_RENDERER_PID', os.getppid()))
            GLib.timeout_add(80, lambda: (os.kill(parent, signal.SIGTERM), False)[1])
            return {'accepted': True}
        if kind == 'theme_positions':
            positions = request.get('positions')
            if not isinstance(positions, list) or len(positions) > len(self.roles):
                raise ValueError('角色位置无效。')
            for item in positions:
                if not isinstance(item, dict) or item.get('window') not in [role[0] for role in self.roles] or not isinstance(item.get('position'), list) or len(item['position']) != 2 or any(type(v) is not int or abs(v) > 1000000 for v in item['position']):
                    raise ValueError('角色位置无效。')
            self.display.error_trap_push()
            try:
                for item in positions:
                    window = GdkX11.X11Window.foreign_new_for_display(self.display, item['window'])
                    if window and not window.is_destroyed():
                        window.move(*item['position'])
                self.display.sync()
            finally:
                self.display.error_trap_pop()
            return {'accepted': True}
        if kind == 'collect_garbage' and os.environ.get('MOCHI_BUBBLE_TEST') == '1':
            import gc
            return {'collected': gc.collect()}
        if kind == 'bubble_themes':
            return {'selected': self.themes.selected, 'themes': self.themes.choices}
        if kind == 'set_bubble_theme':
            self.select_theme(request.get('theme'))
            return {'accepted': True, 'theme': self.themes.selected}
        if os.environ.get('MOCHI_BUBBLE_TEST') == '1' and kind == 'inspect':
            speech = self.speech.inspect()
            plugin = self.plugin_runtime.instances.get(self.panel_owner) or next(iter(self.plugin_runtime.instances.values()), None)
            extra = plugin.inspect() if plugin else {}
            active = self.panel.window if self.presentation == 'panel' and self.panel else self.speech.window
            return {**extra, 'visible': active.get_visible(), 'role': self.primary_id,
                    'window': active.get_window().get_xid() if active.get_realized() else None,
                    'position': self.current_position, 'size': list(active.get_size()),
                    'following': self.follow_source is not None,
                    'controls': self.panel.inspect() if self.presentation == 'panel' and self.panel else speech['controls'],
                    'presentation': self.presentation, 'panel_visible': bool(self.panel and self.panel.window.get_visible()), 'speech': speech,
                    'menu': self.menu.inspect(), 'context_menu': self.context_menu.inspect(), 'plugin_status': self.plugin_runtime.status(),
                    'panel_reply_visible': bool(self.panel and getattr(self.panel, 'composer', None) and self.panel.composer.get_visible()),
                    'panel_owner': self.panel_owner,
                    'active_message': self.messages.active.id if self.messages.active else None,
                    'system_notifications': self.system_notifications.history,
                    'messages': [{'id': m.id, 'source': m.source, 'owner': m.owner, 'state': m.state.value,
                                  'text': m.content.text, 'priority': m.priority} for m in self.messages.messages],
                    'expiry_pending': self.expiry_source is not None, 'speech_text': speech['text']}
        self.require_primary()
        if kind == 'notify':
            message_id = self.notifications.publish(request.get('text'), request.get('title', '通知'),
                                                    request.get('ttl_ms', 8000), request.get('priority', 10))
            # A manual management panel is never displaced by a notification.
            if not (self.panel and self.panel.window.get_visible()):
                self.show('speech')
            return {'accepted': True, 'message_id': message_id}
        return self.plugin_runtime.request(request)

    def quit(self):
        self.shutting_down = True
        self.close_all()
        Gtk.main_quit()
        return False

    def cleanup(self):
        self.shutting_down = True
        self.close_all()
        for state in list(self.clients.values()):
            self.close_client(state)
        self.server.close()
        self.sock_path.unlink(missing_ok=True)
        self.plugin_runtime.shutdown()
        self.menu.destroy()
        self.context_menu.destroy()


if __name__ == '__main__':
    if not os.environ.get('MOCHI_SESSION_FD'):
        os.environ.setdefault('MOCHI_RENDERER_PID', str(os.getppid()))
        from session_launch import launch
        raise SystemExit(launch('host', sys.argv[1:]))
    host = BubbleHost(json.loads(sys.argv[1]))
    restore = os.environ.pop('MOCHI_RESTORE_PANEL', '')
    if restore in host.plugin_runtime.instances:
        GLib.idle_add(lambda: (host.plugin_runtime.instances[restore].open(), False)[1])
    try:
        Gtk.main()
    finally:
        host.cleanup()
