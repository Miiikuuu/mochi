"""Package action inventory and per-Theme overrides; original assets stay read-only."""
from copy import deepcopy
from functools import lru_cache
from pathlib import Path
import tomllib

NAMES = {'idle': 'Stand', 'wave': 'Interact', 'walk': 'Walk', 'sit': 'Sit',
         'sleep': 'Sleep', 'idle_sit': 'Sit & swing', 'hover': 'Hover',
         'drag': 'Drag & recover', 'gaze': 'Look around', 'shake': 'Shake & recover', 'jump': 'Jump',
         'work': 'Work'}
TRIGGERS = {'package': 'Default', 'always': 'Always', 'manual': 'Preview only',
            'click': 'Click', 'hover': 'Pointer enters', 'keyboard': 'Arrow keys',
            'idle': 'When idle', 'pointer': 'Pointer nearby', 'drag': 'While dragging',
            'shake': 'While shaking', 'space': 'Space', 'task': 'While a task runs',
            **{'key_'+str(n): str(n)+' key' for n in range(6, 10)}}
KEYS = ['key_'+str(n) for n in range(6, 10)]


@lru_cache(maxsize=128)
def inventory(path, stamp):
    root = Path(path).resolve()
    manifest = tomllib.loads((root/'manifest.toml').read_text())
    component = next((c for c in manifest['components'] if c['id'] in
                      ('mochi.appearance.spine', 'mochi.appearance.animated-sprite')), None)
    if not component:
        return ()
    source = (root/component['config']['file']).resolve()
    if not source.is_relative_to(root):
        raise ValueError('动作定义必须位于角色包内。')
    data = tomllib.loads(source.read_text())
    spine = component['id'] == 'mochi.appearance.spine'
    result = []
    for key, clip in data['clips'].items():
        default = ('always' if key == data.get('default', 'idle') else
                   'idle' if key == data.get('idle_action') else
                   'task' if key == 'work' else 'package')
        choices = (['always'] if default == 'always' else
                   ['task', 'manual', *KEYS] if default == 'task' else
                   ['package', 'keyboard', 'idle', 'manual'] if key == 'walk' and spine else
                   ['idle', 'manual', *KEYS] if default == 'idle' else
                   ['package', 'manual', 'click', 'hover', *KEYS])
        result.append({'id': key, 'name': NAMES.get(key, key.replace('_', ' ').title()),
                       'kind': 'clip', 'source': clip.get('animation', key),
                       'mode': clip.get('mode', 'loop'), 'required': default == 'always',
                       'default': default, 'choices': choices})
    for key, present, trigger in [('gaze', data.get('follow'), 'pointer'),
                                  ('drag', data.get('drag') or data.get('pull'), 'drag'),
                                  ('shake', data.get('shake'), 'shake'), ('jump', data.get('jump'), 'space')]:
        if present:
            result.append({'id': key, 'name': NAMES[key], 'kind': 'interaction', 'source': key,
                           'mode': 'gesture', 'required': False, 'default': trigger,
                           'choices': [trigger, 'manual', *KEYS] if key == 'jump' else [trigger, 'manual']})
    return tuple(result)


def actions(item):
    root = Path(item['path'])
    # Package-local configuration edits invalidate the small metadata cache.
    stamp = tuple((p.name, p.stat().st_mtime_ns, p.stat().st_size) for p in sorted(root.glob('*.toml')))
    return deepcopy(list(inventory(str(root), stamp)))


def settings(theme, character, action):
    return {'enabled': True, 'trigger': action['default'],
            **theme.get('actions', {}).get(character, {}).get(action['id'], {})}


def bindings(action, setting):
    if not setting['enabled'] or setting['trigger'] == 'manual':
        return []
    trigger = setting['trigger']
    if trigger == 'package':
        return {'wave': ['click'], 'hover': ['hover']}.get(action['id'], [])
    return [trigger] if trigger in ['click', 'hover', *KEYS] else []


def validate_schema(overrides):
    """Validate saved data without opening an optional character package."""
    import re
    if not isinstance(overrides, dict) or len(overrides) > 128:
        raise ValueError('Invalid action settings.')
    for character, values in overrides.items():
        if not isinstance(character, str) or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}', character) or not isinstance(values, dict) or len(values) > 128:
            raise ValueError('Invalid action character.')
        for key, setting in values.items():
            if (not isinstance(key, str) or not 1 <= len(key) <= 128 or
                    not isinstance(setting, dict) or set(setting) != {'enabled', 'trigger'} or
                    type(setting['enabled']) is not bool or setting['trigger'] not in TRIGGERS):
                raise ValueError('Invalid action trigger settings.')
    return deepcopy(overrides)


def validate(overrides, catalog):
    if not isinstance(overrides, dict) or len(overrides) > 128:
        raise ValueError('动作设置无效。')
    for character, values in overrides.items():
        if character not in catalog or not isinstance(values, dict):
            raise ValueError('动作设置中的角色不存在。')
        records = {a['id']: a for a in actions(catalog[character])}
        if any(key not in records for key in values):
            raise ValueError('角色包不包含这个动作。')
        occupied = {}
        for key, record in records.items():
            setting = values.get(key, {'enabled': True, 'trigger': record['default']})
            if (not isinstance(setting, dict) or set(setting) != {'enabled', 'trigger'} or
                    type(setting['enabled']) is not bool or setting['trigger'] not in record['choices']):
                raise ValueError('动作触发设置无效。')
            if record['required'] and (not setting['enabled'] or setting['trigger'] != 'always'):
                raise ValueError('基础站立动作必须保留。')
            for binding in bindings(record, setting):
                if binding in occupied:
                    raise ValueError(f"{TRIGGERS[binding]} 已用于 {occupied[binding]}，请先更改该动作的触发方式。")
                occupied[binding] = record['name']
    return deepcopy(overrides)


def runtime_policy(theme, keys, catalog):
    return [{a['id']: {**settings(theme, key, a), 'kind': a['kind']} for a in actions(catalog[key])} for key in keys]
