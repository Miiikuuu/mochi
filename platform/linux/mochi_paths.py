"""One relocatable layout shared by source development and installed programs."""
from pathlib import Path

LIBEXEC = Path(__file__).resolve().parents[2]
INSTALLED = (LIBEXEC/'installation.json').is_file()
PUBLIC_SOURCE = (LIBEXEC/'PUBLIC_SOURCE_DISTRIBUTION.json').is_file()
PREFIX = LIBEXEC.parents[1] if INSTALLED else LIBEXEC
DATA = PREFIX/'share/mochi' if INSTALLED else LIBEXEC
LOCALE = PREFIX/'share/locale' if INSTALLED else LIBEXEC/'locale'
RENDERER = LIBEXEC/'bin/mochi-desktop' if INSTALLED else LIBEXEC/'target/release/mochi-desktop'

# Earlier full staging installations did not carry a feature flag.
import json
SPINE_SUPPORTED = not PUBLIC_SOURCE and (not INSTALLED or json.loads((LIBEXEC/'installation.json').read_text()).get('spine', True))
