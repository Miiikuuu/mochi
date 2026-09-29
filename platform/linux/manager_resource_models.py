"""Toolkit-free resource snapshots and decoded thumbnail values."""
from dataclasses import dataclass, field


@dataclass(frozen=True)
class Thumbnail:
    pixels: bytes
    width: int
    height: int
    rowstride: int


@dataclass(frozen=True)
class ResourceCatalogSnapshot:
    revision: int
    characters: dict
    bubbles: dict
    plugins: dict
    previews: dict
    thumbnails: dict
    index: dict
    dependencies: dict = field(default_factory=dict)
    theme_revision: int = 0
    plugin_directories: tuple = ()
    runtime_dependencies: tuple = ()
    runtime_error: str | None = None

