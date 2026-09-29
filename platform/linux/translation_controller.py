"""Translation producer. Owns task data and actions, without importing GTK."""
import json
import os
from pathlib import Path
import re
import time
from messages import Action, Content, MessageState

LANGUAGES = {'zh-CN': '简体中文', 'en': 'English', 'ja': '日本語', 'ko': '한국어',
             'fr': 'Français', 'de': 'Deutsch', 'es': 'Español'}
DEFAULT_MODEL = 'gpt-5.6-luna'


def valid_model(value):
    return isinstance(value, str) and re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._:/-]{0,127}', value) is not None


def validate_text(text):
    if not isinstance(text, str) or len(text.encode('utf-16-le')) // 2 > 12000:
        raise ValueError('请选择不超过 12000 个字符的文字。')


class TranslationController:
    source_id = 'translation'

    def __init__(self, messages, transport, preferences_path, clipboard, open_panel):
        self.messages, self.transport = messages, transport
        self.preferences_path = Path(preferences_path)
        self.clipboard, self.open_panel = clipboard, open_panel
        self.listeners = []
        self.source = self.text = ''
        self.status = '选中文字后，右键「用 Mochi 翻译」'
        self.state = MessageState.INFO
        self.actual_model = None
        self.message_id = self.job = self.catalog_job = None
        self.copied = False
        self.preferences = {'target': 'zh-CN', 'model': DEFAULT_MODEL, 'fast': False}
        self.catalog = None
        self.catalog_time = 0
        self.catalog_status = '从已登录的 Codex 获取模型'
        try:
            data = json.loads(self.preferences_path.read_text())
            self.validate_options(data.get('target'), data.get('model'), data.get('fast'))
            self.preferences.update({k: data[k] for k in self.preferences if data.get(k) is not None})
        except (OSError, ValueError, TypeError, AttributeError):
            pass
        messages.register(self.source_id, self.action, self.removed)

    @staticmethod
    def validate_options(target=None, model=None, fast=None):
        if (target is not None and (not isinstance(target, str) or target not in LANGUAGES) or
                model is not None and not valid_model(model) or fast is not None and type(fast) is not bool):
            raise ValueError('不支持的语言、模型或速度设置。')

    @property
    def busy(self):
        return self.state == MessageState.RUNNING

    @property
    def model_names(self):
        names = {key: value['name'] for key, value in (self.catalog or {}).items()}
        model = self.preferences['model']
        if model not in names:
            names[model] = model + ('（当前不可用）' if self.catalog is not None else '')
        return names

    def subscribe(self, callback):
        self.listeners.append(callback)

    def changed(self):
        for callback in tuple(self.listeners):
            callback()

    def configure(self, target=None, model=None, fast=None):
        self.validate_options(target, model, fast)
        self.preferences.update({k: v for k, v in {'target': target, 'model': model, 'fast': fast}.items() if v is not None})
        try:
            self.preferences_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            tmp = self.preferences_path.with_suffix('.tmp')
            with tmp.open('w') as out:
                os.chmod(tmp, 0o600)
                json.dump(self.preferences, out)
            tmp.replace(self.preferences_path)
        except OSError:
            self.catalog_status = '设置未能保存，请检查配置目录。'
        self.changed()

    def set_source(self, text):
        self.source = text  # Validate at submission, so editing stays unrestricted.

    def prepare_panel(self):
        if not self.source:
            self.state, self.status = MessageState.INFO, ''
        self.changed()

    def content(self):
        actions = [Action('copy', '已复制' if self.copied else '复制译文', bool(self.text))]
        if self.source and not self.busy and not self.text:
            actions.append(Action('retry', '重试'))
        actions.append(Action('expand', '展开 ↗', align='end', tooltip='打开完整翻译面板'))
        return Content('翻译', self.text, self.status, self.busy or not bool(self.text), tuple(actions))

    def publish_update(self):
        self.changed()
        if self.message_id:
            self.messages.update(self.source_id, self.message_id, content=self.content(), state=self.state)

    def start(self, text=None):
        source = self.source if text is None else text
        try:
            validate_text(source)
        except ValueError as error:
            self.state, self.status = MessageState.ERROR, str(error)
            self.changed()
            raise
        source = source.strip()
        if not source:
            self.state, self.status = MessageState.ERROR, '请选择或输入 1–12000 个字符。'
            self.changed()
            return
        # Replace the source's slot in one manager update. Clearing the old
        # presentation first would discard the skin's outgoing-message motion.
        self.stop_worker()
        self.message_id = None
        self.source, self.text = source, ''
        self.actual_model = None
        self.copied = False
        self.state, self.status = MessageState.RUNNING, '正在连接翻译…'
        try:
            self.message_id = self.messages.publish(self.source_id, self.content(), self.state, priority=50, slot='task')
        except ValueError as error:
            self.state, self.status = MessageState.ERROR, str(error)
            self.changed()
            raise
        message_id = self.message_id
        self.changed()
        try:
            self.job = self.transport.start({'type': 'translate', 'text': source, **self.preferences},
                                            lambda event: self.event(message_id, event),
                                            lambda error: self.failed(message_id, error))
        except OSError:
            self.failed(message_id, '翻译插件未就绪，请运行 scripts/install-mochi-translate.sh。')

    def stop_worker(self, reuse=False):
        if self.job:
            (self.transport.release if reuse else self.transport.stop)(self.job)
            self.job = None

    def event(self, message_id, event):
        if message_id != self.message_id or not self.busy:
            return
        if event.get('type') == 'status':
            self.status = '正在翻译…' if event.get('status') == 'translating' else '正在连接翻译…'
        elif event.get('type') == 'result':
            text, model, target = event['translated_text'], event['model'], event['target_language']
            if (not isinstance(text, str) or not text.strip() or not valid_model(model) or target not in LANGUAGES
                    or not isinstance(event.get('detected_language'), str) or len(event['detected_language']) > 120):
                raise ValueError('译文数据无效，请重试。')
            # Validate content before committing it to either view.
            Content('翻译', text)
            self.text, self.actual_model = text, model
            speed = ' · Fast' if event.get('fast') else ''
            self.status = f"{event['detected_language']} → {LANGUAGES[target]} · {self.model_names.get(model, model)}{speed}"
            self.state = MessageState.SUCCESS
            self.stop_worker(reuse=True)
        elif event.get('type') == 'error':
            self.failed(message_id, event.get('error', '翻译失败。'))
            return
        else:
            raise ValueError('翻译返回了未知事件。')
        self.publish_update()

    def failed(self, message_id, error):
        if message_id != self.message_id or not self.busy:
            return
        self.stop_worker()
        self.state, self.status = MessageState.ERROR, str(error)[:2000]
        self.publish_update()

    def cancel(self):
        if self.busy:
            self.stop_worker()
            self.state, self.status = MessageState.CANCELLED, '已取消'
            self.publish_update()

    def action(self, message_id, action_id):
        if message_id != self.message_id:
            return
        if action_id == 'copy' and self.text:
            self.clipboard(self.text)
            self.copied, self.status = True, '译文已复制'
            self.publish_update()
        elif action_id == 'retry':
            self.start()
        elif action_id == 'expand':
            self.open_panel()

    def removed(self, message_id, _reason):
        if message_id == self.message_id:
            self.stop_worker()
            self.message_id = None
            self.source = self.text = ''
            self.actual_model = None
            self.state, self.status = MessageState.INFO, ''
            self.copied = False
            self.changed()

    def dismiss(self, reason='closed'):
        if self.message_id:
            self.messages.dismiss(self.message_id, reason)
        else:
            self.source = self.text = ''
            self.actual_model = None
            self.state, self.status = MessageState.INFO, ''
            self.changed()

    def stop_catalog(self, reuse=False):
        if self.catalog_job:
            (self.transport.release if reuse else self.transport.stop)(self.catalog_job)
            self.catalog_job = None
            self.changed()

    def refresh_models(self):
        if self.catalog_job:
            return
        self.catalog_status = '正在读取 Codex 模型…'
        try:
            self.catalog_job = self.transport.start({'type': 'list_models'}, self.catalog_event, self.catalog_error, 18000)
        except OSError:
            self.catalog_error('无法读取模型，请检查 Codex 后刷新。')
        self.changed()

    def catalog_error(self, error):
        self.stop_catalog()
        self.catalog_status = str(error)[:500]
        self.changed()

    def catalog_event(self, event):
        if event.get('type') == 'error':
            self.catalog_error(event.get('error', '无法读取模型列表。'))
            return
        models = event['models']
        if event.get('type') != 'models' or not isinstance(models, list) or not models or any(
                not isinstance(m, dict) or not valid_model(m.get('id')) or not isinstance(m.get('name'), str) or len(m['name']) > 160
                or m.get('fast_tier') not in (None, 'fast', 'priority') for m in models):
            raise ValueError('模型列表无效，请刷新重试。')
        self.catalog = {m['id']: m for m in models}
        self.catalog_time = time.monotonic()
        self.catalog_status = f'已同步 {len(models)} 个 Codex 模型'
        self.stop_catalog(reuse=True)
        self.changed()
