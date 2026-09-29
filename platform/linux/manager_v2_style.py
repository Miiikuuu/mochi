"""Read the shared Manager tokens and GTK4 stylesheet outside the view."""
import json
from pathlib import Path

from theme_store import ROOT


def tokens():
    return json.loads((ROOT/'assets/manager/design-tokens.json').read_text())


def layout_tokens():
    return tokens()['v2']


def stylesheet(dark=False):
    theme_tokens = tokens()
    colors = '\n'.join('@define-color mc_'+key.replace('-', '_')+' '+value['dark' if dark else 'light']+';'
                       for key, value in theme_tokens['colors'].items())
    css = (ROOT/'assets/manager/v2.css').read_text()
    return (colors+'\n'+css).encode()
