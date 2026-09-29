"""Localized disclosure shared by both Manager approval dialogs."""
from manager_i18n import _


def review_details(review):
    details = [review['name'], _('Runtime')+': '+review['runtime'],
               _('Capabilities')+': '+(', '.join(review['capabilities']) or _('None')),
               _('Isolation')+': '+_(review['isolation'])]
    if review['isolation'] == 'unsandboxed':
        details.append(_('Runs with your user permissions.'))
    details.append(_(review.get('session_isolation', 'Session isolation unavailable')))
    details.append('SHA-256: '+review['package_sha256'])
    return '\n'.join(details)
