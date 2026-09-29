"""Cargo workspace is the only source version; installation carries that value."""
import json
import tomllib
from mochi_paths import INSTALLED, LIBEXEC

VERSION = (json.loads((LIBEXEC/'installation.json').read_text())['version'] if INSTALLED
           else tomllib.loads((LIBEXEC/'Cargo.toml').read_text())['workspace']['package']['version'])
