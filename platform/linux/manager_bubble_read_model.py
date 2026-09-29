"""Small, sanitized Bubble Theme palette for static GTK4 previews."""
from dataclasses import dataclass
import re
from resource_catalog import read_bounded


_COLOR = re.compile(r'#[0-9a-fA-F]{3}(?:[0-9a-fA-F]{3})?\b')
_CARD = re.compile(r'\.mochi-speech-card\s*\{([^}]*)\}', re.S)


@dataclass(frozen=True)
class BubblePreviewReadModel:
    key: str
    name: str
    background: str
    foreground: str
    border: str
    user_background: str


def _property(block, name):
    match = re.search(r'(?:^|;)\s*'+re.escape(name)+r'\s*:\s*([^;]+)', block)
    color = _COLOR.search(match.group(1)) if match else None
    return color.group(0) if color else None


def bubble_preview_model(bubbles, key):
    path = bubbles.resolve(key)
    css = read_bounded(path/'speech.css', 256*1024).decode('utf-8')
    card = _CARD.search(css)
    block = card.group(1) if card else ''
    background = _property(block, 'background') or '#f7f8f4'
    foreground = _property(block, 'color') or '#29352f'
    border = _property(block, 'border') or '#b9cbbd'
    controls = path/'controls.css'
    user_background = (_property(read_bounded(controls, 256*1024).decode('utf-8'), 'background') if controls.is_file() else None) or background
    return BubblePreviewReadModel(key, bubbles.choices.get(key, key), background, foreground, border, user_background)
