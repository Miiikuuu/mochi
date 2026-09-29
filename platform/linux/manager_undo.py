"""Toolkit-free, revision-scoped Manager undo commands."""
from dataclasses import dataclass
from typing import Any


class UndoExpired(ValueError):
    pass


@dataclass(frozen=True)
class UndoCommand:
    theme_id: str
    scope: str
    before: tuple[Any, ...]
    after: tuple[Any, ...]
    store_revision: int

    @classmethod
    def cast(cls, before, after, revision):
        return cls(after['id'], 'cast',
                   (tuple(before['characters']), before['primary']),
                   (tuple(after['characters']), after['primary']), revision)

    @classmethod
    def features(cls, before, after, revision):
        return cls(after['id'], 'features',
                   (tuple(before['plugins']),), (tuple(after['plugins']),), revision)

    @property
    def before_characters(self):
        return self.before[0] if self.scope == 'cast' else None

    @property
    def before_primary(self):
        return self.before[1] if self.scope == 'cast' else None

    @property
    def after_characters(self):
        return self.after[0] if self.scope == 'cast' else None

    @property
    def after_primary(self):
        return self.after[1] if self.scope == 'cast' else None

    def validate(self, theme, revision):
        if theme['id'] != self.theme_id or revision != self.store_revision:
            raise UndoExpired('The Theme changed after this action.')
        if self.scope == 'cast':
            current = (tuple(theme['characters']), theme['primary'])
        elif self.scope == 'features':
            current = (tuple(theme['plugins']),)
        else:
            raise UndoExpired('Unknown undo scope.')
        if current != self.after:
            raise UndoExpired('The action was superseded by newer edits.')

    def changes(self):
        if self.scope == 'cast':
            return {'characters': list(self.before_characters), 'primary': self.before_primary}
        if self.scope == 'features':
            return {'plugins': list(self.before[0])}
        raise UndoExpired('Unknown undo scope.')
