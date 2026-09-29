#!/usr/bin/env python3
"""Single user-facing Manager command resolver. GTK3 is a temporary Legacy escape hatch."""
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]


def manager_command(legacy=False):
    return ['/usr/bin/python3', str(Path(__file__).resolve()), *(['--legacy'] if legacy else [])]


def resolved_command(legacy=False):
    return ['/usr/bin/python3', str(Path(__file__).with_name('manager.py' if legacy else 'manager_v2.py'))]


def desktop_exec():
    def quote(value):
        return '"'+value.replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')+'"'
    return ' '.join(quote(value) for value in manager_command())


if __name__ == '__main__':
    args = sys.argv[1:]
    legacy = '--legacy' in args
    args = [arg for arg in args if arg != '--legacy']
    if args == ['--print-command']:
        print(json.dumps(resolved_command(legacy)))
    else:
        if legacy:
            os.environ['MOCHI_MANAGER_LEGACY'] = '1'
        else:
            os.environ.pop('MOCHI_MANAGER_LEGACY', None)
        command = resolved_command(legacy)
        if args:
            raise SystemExit('Unsupported Manager launcher arguments.')
        from session_launch import launch
        raise SystemExit(launch('legacy' if legacy else 'manager'))
