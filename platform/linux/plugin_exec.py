"""Minimal post-namespace entry: no Catalog writes inside the read-only mount."""
import os
import sys
from plugin_launch import restrict_writes
from pathlib import Path
if __name__ == '__main__':
    state, executable, entry, origin = sys.argv[1:]
    roots = None if origin == 'bundled' else [
        '/usr', '/lib', '/lib64', '/etc', '/dev', str(Path('/proc/self').resolve()),
        str(Path(entry).parent), str(Path(executable).resolve().parent.parent)]
    restrict_writes(state, roots)
    os.execve(executable, [executable, entry], os.environ)
