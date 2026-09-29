"""Plugin lifecycle, independent process adapters and source-scoped message routing."""
from dataclasses import replace
from datetime import datetime, timezone
import importlib
import os
import shutil
from gi.repository import Gtk, GLib
from messages import Action, Content, MessageState, Reply
from session_client import ReplyBindingAdapter, ensure_sessiond
from native_jobs import NativeJobs
from plugin_catalog import settings_for
from plugin_launch import launch_plan
from plugin_panel import PluginPanel


def content_from_json(value, name):
    if not isinstance(value, dict) or set(value)-{'title', 'text', 'status', 'show_status', 'actions', 'kind'}:
        raise ValueError('插件内容格式无效。')
    actions = value.get('actions', [])
    if not isinstance(actions, list) or len(actions) > 6 or any(not isinstance(a, dict) for a in actions):
        raise ValueError('插件按钮格式无效。')
    return Content(**{**{'title': name}, **value, 'actions': tuple(Action(**action) for action in actions)})


class ProcessPlugin:
    request_types = ()

    def __init__(self, host, spec, catalog):
        self.host, self.spec, self.key = host, spec, spec['id']
        self.enabled = False
        self.source = 'plugin:'+self.key
        self.settings = settings_for(spec)
        self.theme_id = None
        self.job = None
        self.failed = False
        self.last_event = None
        self.slots = {}
        self.active_slots = set()
        self.external_id = None
        self.external_content = None
        self.external_channel = None
        self.external_outgoing = False
        self.mailbox = None
        self._binding_suspended = True
        self._binding_closed = False
        self.reply_timer = None
        plan = launch_plan(spec, catalog.resources.directory)
        self.transport = NativeJobs(plan.command, idle_timeout_ms=30000,
                                    cwd=plan.cwd, env=plan.environment, process_group=True,
                                    before_start=lambda: catalog.require_approved(self.key),
                                    stderr_path=plan.state_bind_rw+'/runtime.log')
        self.panel = PluginPanel(spec['name'], host.close_panel, self.panel_action, host.follow_once, self.submit_reply)
        self.view = Content(spec['name'], spec['description'])
        self.panel.render(self.view)
        host.messages.register(self.source, self.message_action, self.removed, self.message_reply)
        if spec.get('builtin') and self.key == 'chatgpt' and 'external_replies' in spec['capabilities']:
            ensure_sessiond()
            self.mailbox = ReplyBindingAdapter(self.binding_changed, lambda fn, *args: GLib.idle_add(fn, *args), self.key, self.prepare_session_relaunch)

    def prepare_session_relaunch(self):
        if self.host.panel and self.host.panel.window.get_visible():
            os.environ['MOCHI_RESTORE_PANEL'] = self.host.panel_owner or ''

    @property
    def busy(self):
        return self.job is not None

    def configure(self, theme):
        settings = settings_for(self.spec, (theme or {}).get('plugin_settings', {}).get(self.key))
        theme_id = (theme or {}).get('id', 'current-desktop')
        if self.settings != settings or self.theme_id != theme_id:
            self.stop()
        self.settings, self.theme_id = settings, theme_id

    def open(self):
        if self.external_content and not self.busy:
            self.view = self.external_content
            self.panel.render(self.view)
            self.host.show_panel(self)
            self.panel.window.present_with_time(Gtk.get_current_event_time())
            return
        self.host.show_panel(self)
        self.panel.window.present_with_time(Gtk.get_current_event_time())
        if not self.busy:
            self.start('open')

    def start(self, kind, **fields):
        if not self.enabled or self.busy:
            return
        self.failed = False
        self.active_slots = set()
        self.view = replace(self.view, status='加载中…', show_status=True)
        self.panel.render(self.view)
        try:
            self.job = self.transport.start({'type': kind, 'api_version': 1, 'plugin_id': self.key,
                                             'theme_id': self.theme_id, 'settings': self.settings, **fields},
                                            self.received, self.error, self.spec['timeout_ms'])
        except (OSError, ValueError) as error:
            self.error(str(error))

    def received(self, event):
        kind = event.get('type')
        if kind == 'done':
            if any((message := self.host.messages.get(self.slots.get(slot))) and message.state == MessageState.RUNNING
                   for slot in self.active_slots):
                raise ValueError('插件结束请求前需要完成进行中的消息。')
            self.transport.release(self.job)
            self.job = None
            self.active_slots.clear()
            self.view = replace(self.view, status='')
            if self.external_content:
                self.view = self.external_content
            self.panel.render(self.view)
            return
        if kind == 'error':
            self.error(str(event.get('error', '插件请求失败。'))[:1000])
            return
        if kind == 'view':
            self.view = content_from_json(event.get('content'), self.spec['name'])
            self.panel.render(self.view)
            return
        if kind not in ('message', 'update', 'dismiss'):
            raise ValueError('未知的插件响应。')
        slot = event.get('slot')
        if not isinstance(slot, str) or not 1 <= len(slot) <= 80:
            raise ValueError('插件消息需要有效的 slot。')
        identity = self.slots.get(slot)
        if kind == 'dismiss':
            if identity:
                self.host.messages.dismiss(identity, 'plugin_closed')
            return
        content = content_from_json(event.get('content'), self.spec['name'])
        state = MessageState(event.get('state', 'info'))
        if kind == 'update':
            # A late update cannot resurrect a dismissed message.
            if identity:
                self.host.messages.update(self.source, identity, content=content, state=state)
        else:
            self.slots[slot] = self.host.messages.publish(self.source, content, state, slot=slot,
                                                         priority=event.get('priority', 10), ttl_ms=event.get('ttl_ms'))
        self.active_slots.add(slot)
        if self.host.panel is None or not self.host.panel.window.get_visible():
            self.host.show('speech')

    def panel_action(self, action):
        if not any(a.id == action and a.enabled for a in self.view.actions):
            return
        if self.view is self.external_content and self.external_action(self.external_id, action, panel=True):
            return
        self.action(action, None)

    def message_action(self, identity, action):
        if self.external_action(identity, action):
            return
        slot = next((s for s, mid in self.slots.items() if mid == identity), None)
        if slot is not None:
            self.action(action, slot)

    def action(self, action, slot):
        if action == 'cancel':
            self.cancel()
        elif self.failed and action == 'retry':
            self.open()
        else:
            self.start('action', action=action, slot=slot)

    def removed(self, identity, reason):
        if identity == self.external_id:
            self.external_id = None
        slot = next((s for s, mid in self.slots.items() if mid == identity), None)
        if slot is not None:
            del self.slots[slot]
            if slot in self.active_slots and reason in ('closed', 'expired'):
                self.cancel()

    def receive_external(self, request, *, outgoing=False, binding_notice=False):
        text, title = request.get('text'), request.get('title', self.spec['name'])
        channel = request.get('conversation_key')
        if channel and self.mailbox:
            self.mailbox.refresh()
        if channel is not None and (self.mailbox is None or channel != self.mailbox.active or 'external_replies' not in self.spec['capabilities']):
            raise ValueError('此对话尚未连接桌宠，请先连接对应的 ChatGPT 卡片。')
        if not isinstance(text, str) or not text.strip() or len(text) > 12000:
            raise ValueError('消息需要 1–12000 个字符。')
        if channel and not outgoing and not binding_notice:
            self.mailbox.observed(channel)
        self.last_event = datetime.now(timezone.utc).isoformat(timespec="seconds")
        state = self.mailbox.snapshot() if channel else None
        content = Content(title, text, kind='chat', actions=(Action('external_copy', '复制'),
                          Action('external_dismiss', '关闭', align='end')),
                          reply=Reply(channel, state['enabled'], state['clear_id'], state['waiting']) if channel else None,
                          status=state['status'] if channel else '',
                          speaker=('user' if outgoing else 'assistant') if channel else '')
        identity = self.host.messages.publish(self.source, content, MessageState.INFO,
                                              priority=60, ttl_ms=request.get('ttl_ms', 60000), slot='external-reply')
        self.external_id = identity
        self.external_content = content
        self.external_channel = channel
        self.external_outgoing = outgoing
        self.refresh_reply()
        content = self.external_content
        if self.host.panel_owner == self.key and self.panel.window.get_visible() and not self.busy:
            self.view = content
            self.panel.render(content)
        elif not (self.host.panel and self.host.panel.window.get_visible()):
            self.host.show('speech')
        return {'accepted': True, 'message_id': identity}

    def message_reply(self, identity, text):
        if identity != self.external_id:
            raise ValueError('这条消息已被替换。')
        return self.submit_reply(text)

    def submit_reply(self, text):
        identity = self.mailbox.submit(self.external_channel, text)
        # A retry updates the same outgoing bubble. A new turn gets its own
        # message identity so presentation can move the previous turn upward.
        if not self.external_outgoing or not self.external_content or self.external_content.text != text:
            self.receive_external({'text': text, 'title': '我', 'conversation_key': self.external_channel,
                                   'ttl_ms': None}, outgoing=True)
        self.refresh_reply()
        return identity

    def refresh_reply(self):
        if not self.external_content or not self.external_channel:
            return
        state = self.mailbox.snapshot()
        if state['key'] != self.external_channel:
            return
        content = replace(self.external_content, reply=Reply(state['key'], state['enabled'], state['clear_id'], state['waiting']), status=state['status'])
        if content == self.external_content:
            return
        self.external_content = content
        if self.external_id:
            self.host.messages.update(self.source, self.external_id, content=content, state=MessageState.INFO)
        if self.host.panel_owner == self.key and self.panel.window.get_visible():
            self.view = content
            self.panel.render(content)

    def binding_changed(self):
        if self._binding_closed or self._binding_suspended or not self.enabled:
            return
        state = self.mailbox.snapshot()
        if not state['connected']:
            if self.external_channel:
                self.disconnect_reply_binding(reset=False)
            return
        if self.external_channel != state['key']:
            self.receive_external({'text': '已连接此对话，可以直接回复。', 'conversation_key': state['key'], 'ttl_ms': None}, binding_notice=True)
        self.refresh_reply()

    def disconnect_reply_binding(self, reset=True):
        if reset:
            self.mailbox.reset()
        self.external_channel = None
        for message in list(self.host.messages.messages):
            if message.source == self.source and message.content.reply:
                self.host.messages.update(self.source, message.id, content=replace(message.content, reply=None, status='Disconnected', show_status=True), state=MessageState.INFO)
        if self.reply_timer is not None:
            GLib.source_remove(self.reply_timer)
            self.reply_timer = None
        if self.external_content:
            self.external_content = replace(self.external_content, reply=None, status='Disconnected', show_status=True)
            if self.external_id:
                self.host.messages.update(self.source, self.external_id, content=self.external_content, state=MessageState.INFO)
            self.view = self.external_content
        else:
            self.view = replace(self.view, reply=None, status='Disconnected', show_status=True)
        self.panel.render(self.view)
        if hasattr(self.host, 'speech'):
            self.host.speech.forget_source(self.source)

    def reply_bridge(self, request):
        operation = request.get('operation')
        token, client = request.get('token'), request.get('client')
        if operation == 'create':
            return self.mailbox.create()
        if operation == 'attach':
            result = self.mailbox.attach(token, client)
            self.receive_external({'text': '已连接此对话，可以直接回复。', 'conversation_key': result['conversation_key'], 'ttl_ms': None}, binding_notice=True)
        elif operation == 'poll':
            result = self.mailbox.poll(token, client)
        elif operation == 'ack':
            result = self.mailbox.acknowledge(token, client, request.get('reply_id'), request.get('state'))
        elif operation == 'detach':
            result = self.mailbox.detach(token, client)
        else:
            raise ValueError('不支持的回复桥接请求。')
        self.refresh_reply()
        return result

    def external_action(self, identity, action, panel=False):
        if (not panel and (not identity or identity != self.external_id)) or action not in ('external_copy', 'external_dismiss'):
            return False
        if not self.external_content:
            return True
        if action == 'external_copy':
            self.host.copy_text(self.external_content.text)
        else:
            if identity:
                self.host.messages.dismiss(identity)
            self.external_content = None
            if self.host.panel_owner == self.key:
                self.host.close_panel()
        return True

    def settle_messages(self, state, text):
        for slot in tuple(self.active_slots):
            message = self.host.messages.get(self.slots.get(slot))
            if message and message.state == MessageState.RUNNING:
                content = replace(message.content, status=text, show_status=True, actions=())
                self.host.messages.update(self.source, message.id, content=content, state=state)
        self.active_slots.clear()

    def cancel(self):
        self.transport.stop(self.job)
        self.job = None
        self.settle_messages(MessageState.CANCELLED, '已取消')
        self.view = replace(self.view, status='已取消', actions=(Action('retry', '重新打开'),))
        self.failed = True
        self.panel.render(self.view)

    def error(self, error):
        self.transport.stop(self.job)
        self.job = None
        self.failed = True
        self.settle_messages(MessageState.ERROR, error)
        self.view = Content(self.spec['name'], status=error, actions=(Action('retry', '重试'),))
        self.panel.render(self.view)

    def close_panel(self):
        self.panel.window.hide()
        if self.busy:
            self.cancel()

    def stop(self):
        self._binding_suspended = True
        self.close_panel()
        for channel in list(self.transport.channels.values()):
            self.transport.close_channel(channel)
        self.host.messages.remove_source_messages(self.source)
        self.host.speech.forget_source(self.source)
        self.external_content = None
        self.external_channel = None
        if self.reply_timer is not None:
            GLib.source_remove(self.reply_timer)
            self.reply_timer = None
        self.slots.clear()

    def shutdown(self):
        self._binding_closed = True
        if self.mailbox:
            self.mailbox.close()
        self.stop()
        self.transport.shutdown()
        self.panel.window.destroy()

    def status_extra(self):
        return {}

    def inspect(self):
        return {'busy': self.busy, 'text': self.view.text, 'status': self.view.status,
                'worker_pid': self.job['proc'].pid if self.job else None,
                'panel_text': self.view.text, 'settings': self.settings}


class PluginRuntime:
    def __init__(self, host, catalog):
        self.host, self.catalog = host, catalog
        self.instances = {}
        self.enabled = []
        # Bundled adapters keep cheap preferences available while disabled.
        # No process or model request starts during construction.
        for key, spec in catalog.items.items():
            if spec['runtime'] == 'builtin':
                self.get(key)

    def get(self, key):
        previous = self.instances.get(key)
        if previous and previous.spec.get('package_sha256') != self.catalog.items[key].get('package_sha256'):
            previous.shutdown()
            del self.instances[key]
        if key not in self.instances:
            spec = self.catalog.items[key]
            if spec['runtime'] == 'builtin':
                module, name = spec['entry'].split(':')
                factory = getattr(importlib.import_module(module), name)
            else:
                factory = ProcessPlugin
            self.instances[key] = factory(self.host, spec) if spec['runtime'] == 'builtin' else factory(self.host, spec, self.catalog)
        return self.instances[key]

    def apply(self, theme, enabled, legacy=None):
        self.catalog.refresh()
        if any(key not in self.catalog.items for key in enabled):
            raise ValueError('Theme 包含尚未安装的插件。')
        for key in enabled:
            self.catalog.require_approved(key, verify=True)
            spec = self.catalog.items[key]
            settings_for(spec, (theme or {}).get('plugin_settings', {}).get(key))
            if spec['runtime'] != 'builtin' and not shutil.which(spec['runtime']):
                raise ValueError('插件 '+spec['name']+' 需要 '+spec['runtime'])
        # Construct all new views before retiring currently enabled plugins.
        for key in enabled:
            self.get(key)
        for key, plugin in self.instances.items():
            if key not in enabled:
                plugin.enabled = False
                plugin.stop()
        self.enabled = list(enabled)
        for key in enabled:
            plugin = self.get(key)
            plugin.configure(theme)
            plugin.enabled = True
            if isinstance(plugin, ProcessPlugin) and plugin.mailbox:
                plugin._binding_suspended = False
                plugin.binding_changed()
        for key, options in (legacy or {}).items():
            if key in self.catalog.items and hasattr(self.get(key), 'configure_legacy'):
                plugin = self.get(key)
                plugin.configure(theme)
                plugin.configure_legacy(options)

    def entries(self):
        return [(key, self.catalog.items[key]['name'],
                 str(self.catalog.items[key]['root']/self.catalog.items[key]['icon']) if self.catalog.items[key].get('icon') else None)
                for key in self.enabled]

    def open(self, key):
        if key not in self.enabled:
            raise ValueError('当前 Theme 未启用此插件。')
        self.get(key).open()

    def request(self, request):
        for plugin in self.instances.values():
            if request['type'] in plugin.request_types:
                if not plugin.enabled:
                    raise ValueError('当前 Theme 未启用此插件。')
                return plugin.request(request)
        raise ValueError('不支持的桌宠请求。')

    def receive_external(self, request):
        key = request.get('plugin')
        if not isinstance(key, str) or key not in self.enabled:
            raise ValueError('当前 Theme 未启用此插件。')
        plugin = self.get(key)
        if 'external_messages' not in plugin.spec['capabilities'] or not isinstance(plugin, ProcessPlugin):
            raise ValueError('此插件不接收外部消息。')
        return plugin.receive_external(request)

    def reply_bridge(self, request):
        key = request.get('plugin')
        if not isinstance(key, str) or key not in self.enabled:
            raise ValueError('当前 Theme 未启用此插件。')
        plugin = self.get(key)
        if not isinstance(plugin, ProcessPlugin) or 'external_replies' not in plugin.spec['capabilities']:
            raise ValueError('此插件不支持外部回复。')
        return plugin.reply_bridge(request)

    def disconnect_chatgpt(self):
        plugin = self.instances.get('chatgpt')
        if isinstance(plugin, ProcessPlugin):
            plugin.disconnect_reply_binding()
        return {'disconnected': True}

    @property
    def busy(self):
        return any(plugin.busy for plugin in self.instances.values())

    def status(self):
        return {key: {'enabled': key in self.enabled, 'busy': plugin.busy,
                      'version': plugin.spec['version'], 'failed': bool(getattr(plugin, 'failed', False)),
                      'last_event': getattr(plugin, 'last_event', None),
                      'connection': plugin.mailbox.connection_status() if key == 'chatgpt' and isinstance(plugin, ProcessPlugin) else {}}
                for key, plugin in self.instances.items()}

    def extra(self):
        return {key: value for plugin in self.instances.values() for key, value in plugin.status_extra().items()}

    def stop(self):
        for plugin in self.instances.values():
            plugin.stop()

    def shutdown(self):
        for plugin in self.instances.values():
            plugin.shutdown()
