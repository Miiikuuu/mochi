"""Background-only resource parsing with a byte-bounded decoded thumbnail LRU."""
from collections import OrderedDict
from io import BytesIO
from pathlib import Path
import json
from resource_dependencies import ResourceDependencyIndex
from manager_character_read_model import display_name

from bubble_themes import BubbleThemes
from manager_bubble_read_model import BubblePreviewReadModel, bubble_preview_model
from plugin_catalog import PluginCatalog
from resource_catalog import ResourceCatalog, read_bounded
from manager_resource_models import ResourceCatalogSnapshot, Thumbnail


class ThumbnailCache:
    def __init__(self, max_bytes=2*1024*1024):
        self.max_bytes = max_bytes
        self.bytes = 0
        self.entries = OrderedDict()

    def load(self, path):
        if not path:
            return None
        path = Path(path)
        try:
            stat = path.stat()
            key = (str(path), stat.st_mtime_ns, stat.st_size)
            if key in self.entries:
                self.entries.move_to_end(key)
                return self.entries[key]
            from PIL import Image
            data = read_bounded(path, 8*1024*1024)
            with Image.open(BytesIO(data)) as source:
                if source.format != 'PNG' or source.width*source.height > 8_000_000:
                    return None
                source.thumbnail((256, 256), Image.Resampling.LANCZOS)
                image = source.convert('RGBA')
                bounds = image.getbbox()
                if bounds:
                    image = image.crop(bounds)
                thumbnail = Thumbnail(image.tobytes(), image.width, image.height, image.width*4)
            size = len(thumbnail.pixels)
            if size > self.max_bytes:
                return None
            while self.entries and self.bytes + size > self.max_bytes:
                _, previous = self.entries.popitem(last=False)
                self.bytes -= len(previous.pixels)
            self.entries[key] = thumbnail
            self.bytes += size
            return thumbnail
        except (OSError, ValueError):
            return None


class ResourceLoader:
    """Call load only on a worker; adopt its immutable result on the UI thread."""
    def __init__(self, directory, bubble_directory, bubble_preferences, bundled_directory=None):
        self.directory = directory
        self.bubble_directory, self.bubble_preferences = bubble_directory, bubble_preferences
        self.bundled_directory = bundled_directory
        self.thumbnails = ThumbnailCache()
        self.leases = None

    def load(self):
        resources = ResourceCatalog(self.directory)
        bubbles = BubbleThemes(self.bubble_directory, self.bubble_preferences, resources, self.bundled_directory)
        plugins = PluginCatalog(self.directory, resources)
        try:
            runtime = (self.leases or resources.leases).snapshots()
            runtime_error = None
        except (OSError, ValueError, ConnectionError) as error:
            runtime, runtime_error = (), str(error)
        with resources.locked():
            index = resources.read()
            themes_path = Path(self.directory)/'themes.json'
            themes = json.loads(read_bounded(themes_path, 4*1024*1024)) if themes_path.exists() else {}
            dependencies = ResourceDependencyIndex(themes.get('themes', [])).entries
        from resource_metadata import localized
        bubbles._records = {key: localized(item) for key, item in index['bubbles'].items()}
        plugins.items = {key: {**localized(item), 'root': Path(item['root'])} for key, item in index['plugins'].items()}
        characters = {key: {**item, 'display_name': display_name(key, item['name'])}
                      for key, item in index['characters'].items()}
        for kind, manifest in (('characters', 'manifest.toml'), ('bubbles', 'theme.json'), ('plugins', 'mochi-plugin.json')):
            for item in index[kind].values():
                item['adoptable'] = bool(item.get('legacy_bundled') and (Path(item.get('root') or item.get('path') or '')/manifest).is_file())
        for key, item in characters.items():
            item['adoptable'] = index['characters'][key].get('adoptable', False)
        for catalog, kind in ((bubbles._records, 'bubbles'), (plugins.items, 'plugins')):
            for key, item in catalog.items():
                item['adoptable'] = index[kind][key].get('adoptable', False)
        previews = {}
        for key in bubbles.choices:
            try:
                previews[key] = bubble_preview_model(bubbles, key)
            except (OSError, ValueError, UnicodeError):
                previews[key] = fallback_bubble(key, bubbles.choices[key])
        thumbnails, total = {}, 0
        for key, item in index['characters'].items():
            image = self.thumbnails.load(item.get('preview'))
            if image and total + len(image.pixels) <= self.thumbnails.max_bytes:
                thumbnails[key] = image
                total += len(image.pixels)
        directories = {str(plugins.directory), *plugins.errors.keys()}
        for record in plugins.items.values():
            if record.get('origin') == 'local':
                root = Path(record['root'])
                directories.add(str(root))
                for name in record.get('package_files', []):
                    directories.update(str(parent) for parent in (root/name).parents if parent == root or root in parent.parents)
        return ResourceCatalogSnapshot(index['revision'], characters, bubbles._records,
                                       plugins.items, previews, thumbnails, index, dependencies, themes.get('revision', 0), tuple(sorted(directories)), runtime, runtime_error)


def fallback_bubble(key, name):
    return BubblePreviewReadModel(key, name, '#f7f8f4', '#29352f', '#b9cbbd', '#f7f8f4')
