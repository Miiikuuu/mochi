"""Bounded locale-aware manifest text; default fields are the English fallback."""
import os


def validate_i18n(value):
    translations = value.get('i18n', {})
    if not isinstance(translations, dict) or len(translations) > 32:
        raise ValueError('Invalid manifest translations.')
    for locale, fields in translations.items():
        if not isinstance(locale, str) or len(locale) > 32 or not isinstance(fields, dict) or set(fields)-{'name', 'description'}:
            raise ValueError('Invalid manifest locale metadata.')
        for field, content in fields.items():
            if not isinstance(content, str) or not content.strip() or len(content) > (80 if field == 'name' else 500):
                raise ValueError('Invalid localized manifest text.')
    return translations


def localized(record, language=None):
    language = (language or os.environ.get('LANGUAGE') or os.environ.get('LC_ALL') or
                os.environ.get('LC_MESSAGES') or os.environ.get('LANG') or 'en').split(':')[0]
    locale = language.split('.')[0].replace('_', '-').lower()
    values = {key.lower(): value for key, value in record.get('i18n', {}).items()}
    translated = values.get(locale, values.get('zh-cn', {}) if locale.startswith('zh') else values.get(locale.split('-')[0], {}))
    return {**record, **translated}
