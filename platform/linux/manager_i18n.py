"""gettext text selection at the GTK presentation boundary."""
from mochi_paths import LOCALE
import gettext
import os
from pathlib import Path

from manager_state import AppError


_system_language = os.environ.get('LANGUAGE')
_catalog = gettext.translation('mochi', localedir=LOCALE, fallback=True)
def _(message):
    return _catalog.gettext(message)


def configure_language(language='system'):
    global _catalog
    if language == 'system':
        if _system_language is None:
            os.environ.pop('LANGUAGE', None)
        else:
            os.environ['LANGUAGE'] = _system_language
    else:
        os.environ['LANGUAGE'] = language
    _catalog = gettext.translation('mochi', localedir=LOCALE,
                                  languages=None if language == 'system' else [language], fallback=True)

ERROR_MESSAGES = {
    'session.disconnected': 'The ChatGPT session disconnected. Reconnect the conversation card.',
    'resource.in_use': 'This resource is still used by a Theme.',
    'resource.invalid': 'The resource package or operation is invalid.',
    'resource.permission': 'Could not access this resource.',
    'resource.missing': 'The resource is missing.',
    'resource.failed': 'Could not refresh the resource catalog.',
    'plugin.approval_required': 'Review and approve this plugin before enabling it.',
    'plugin.approval_failed': 'Could not approve the plugin. Review its current package.',
    'theme.conflict': 'Theme changed in another window.',
    'theme.invalid': 'Could not save the theme.',
    'theme.failed': 'Could not save the theme.',
    'theme.permission': 'Could not save the theme.',
    'theme.catalog_refresh_failed': 'Theme saved, but the resource list could not be refreshed.',
    'theme.missing': 'The selected theme is missing.',
    'theme.limit': 'This theme supports up to four characters.',
    'theme.primary_required': 'Choose another primary character before disabling this one.',
    'theme.last_character': 'A Theme needs at least one character.',
    'theme.invalid_order': 'Cannot move this character.',
    'runtime.disconnected': 'The desktop runtime disconnected.',
    'runtime.invalid': 'The desktop runtime rejected the command.',
    'runtime.failed': 'The desktop runtime command failed.',
    'status.disconnected': 'Could not reach the desktop runtime.',
    'status.invalid': 'The desktop runtime returned invalid status.',
    'import.invalid': 'The import package is invalid.',
    'import.missing': 'The import file is missing.',
    'import.failed': 'The import failed.',
    'import.selection_failed': 'Theme imported, but it could not be selected.',
    'import.catalog_refresh_failed': 'Import completed, but the resource list could not be refreshed.',
    'translation.invalid': 'The translation settings are invalid.',
    'translation.conflict': 'Translation settings changed in another window.',
    'preview.failed': 'Could not open the preview.',
    'catalog.failed': 'Could not load the model catalog.',
    'catalog.invalid': 'The model catalog is invalid.',
}


def error_parts(error):
    if isinstance(error, AppError):
        message = ERROR_MESSAGES.get(error.code)
        summary = _(message) if message else _(error.fallback)
        details = []
        if error.fallback and error.fallback != summary and error.fallback != message:
            details.append(error.fallback)
        if error.detail and error.detail not in details:
            details.append(error.detail)
        return summary, '\n'.join(details)
    return _('Operation failed.'), str(error)


def error_text(error):
    summary, detail = error_parts(error)
    return summary + (' — ' + detail if detail else '')


def relative_time(value=None, *, seconds=None):
    from datetime import datetime, timezone
    if seconds is None:
        try:
            moment = datetime.fromisoformat(value)
            seconds = (datetime.now(timezone.utc)-moment).total_seconds()
        except (ValueError, TypeError):
            return '—'
    seconds = max(0, int(seconds))
    if seconds < 10:
        return _('Just now')
    if seconds < 60:
        return _('%d seconds ago') % seconds
    if seconds < 3600:
        return _('%d minutes ago') % (seconds//60)
    if seconds < 86400:
        return _('%d hours ago') % (seconds//3600)
    return _('%d days ago') % (seconds//86400)
