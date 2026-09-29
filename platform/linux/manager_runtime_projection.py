"""Toolkit-free interpretation of the desktop snapshot for both Managers."""
from dataclasses import dataclass
from typing import Callable

from manager_state import RuntimeState


@dataclass(frozen=True)
class RuntimePresentation:
    status: str
    action: str
    applied: bool
    same_theme: bool
    degraded: bool
    dirty: bool
    running_theme: dict | None


def project_runtime(runtime: RuntimeState, draft: dict, desired_translation: dict,
                    same_config: Callable[[dict, dict], bool]) -> RuntimePresentation:
    live = runtime.snapshot if isinstance(runtime.snapshot, dict) else None
    current = live.get('theme') if live and isinstance(live.get('theme'), dict) else None
    same = bool(current and current.get('id') == draft.get('id'))
    roles = live.get('roles') if live else None
    expected_roles = current.get('characters') if current else None
    expected_roles = expected_roles if isinstance(expected_roles, list) else []
    roles_valid = isinstance(roles, list) and len(roles) == len(expected_roles) and all(
        isinstance(role, dict) and role.get('alive') is True for role in roles)
    trusted = bool(current and isinstance(current.get('id'), str) and current['id'] and
                   isinstance(current.get('characters'), list) and isinstance(roles, list) and
                   all(isinstance(role, dict) for role in roles))
    degraded = bool(current and not roles_valid)
    translated = bool(live and live.get('translation') == desired_translation)
    applied = bool(same and current.get('name') == draft.get('name') and
                   same_config(current, draft) and translated and roles_valid and not runtime.disconnected)
    dirty = bool(same and not applied)
    if runtime.disconnected:
        status = 'Runtime disconnected'
    elif degraded:
        status = 'Degraded'
    elif same:
        status = 'Running'
    elif current:
        status = 'Another Theme is running'
    else:
        status = 'Stopped'
    action = ('' if runtime.disconnected and not trusted else
              'Restart' if degraded or runtime.disconnected and current else '' if applied else 'Apply changes' if same else
              'Switch to this theme' if current else 'Start theme')
    return RuntimePresentation(status, action, applied, same, degraded, dirty, current)
