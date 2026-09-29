"""Address shared by session-owned desktop UI and local entry points."""
import hashlib
import os
from pathlib import Path


def socket_path():
    if os.environ.get('MOCHI_BUBBLE_SOCKET'):
        return Path(os.environ['MOCHI_BUBBLE_SOCKET'])
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', f'/tmp/mochi-{os.getuid()}'))
    display = hashlib.sha256(os.environ.get('DISPLAY', ':0').encode()).hexdigest()[:12]
    return runtime / 'mochi' / f'bubbles-{display}.sock'
