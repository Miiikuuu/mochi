"""Toolkit-free, locale-aware character labels for Manager presentation."""
from dataclasses import dataclass
import os
from manager_resource_models import Thumbnail


ENGLISH_NAMES = {
    'shanbi': 'Shanbi',
    'angelina_wish': 'Angelina',
    'jacinta': 'Jacinta',
    'timeslot': 'Timeslot',
    'mutsumi': 'Mutsumi',
}


@dataclass(frozen=True)
class CharacterReadModel:
    display_name: str
    original_name: str
    technical_id: str
    thumbnail: Thumbnail | None
    primary: bool
    position: int | None
    order: int | None
    lifecycle: str = "active"


def display_name(key, original_name, language=None):
    language = (language or os.environ.get('LANGUAGE') or os.environ.get('LC_ALL') or
                os.environ.get('LC_MESSAGES') or os.environ.get('LANG') or 'en').split(':')[0].lower()
    if language.startswith('zh'):
        return original_name
    return ENGLISH_NAMES.get(key, original_name)
