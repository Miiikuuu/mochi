"""Bounded, observable Xvfb readiness handshake shared by native checks."""
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import time


def start(output, geometry='1200x900x24', timeout=30):
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    root = Path(__file__).resolve().parents[1]
    binary = shutil.which('Xvfb') or str(root/'target/idle-walk-release/xvfb/root/usr/bin/Xvfb')
    read, write = os.pipe()
    server = None
    began = time.monotonic()
    state = {'stage': 'starting', 'geometry': geometry, 'timeout': timeout}
    try:
        with (output/'xvfb.log').open('w') as log:
            server = subprocess.Popen([binary, '-displayfd', str(write), '-screen', '0', geometry,
                                       '-nolisten', 'tcp'], pass_fds=(write,), stdout=subprocess.DEVNULL, stderr=log)
        os.close(write)
        write = None
        data = bytearray()
        while time.monotonic()-began < timeout:
            if server.poll() is not None:
                raise RuntimeError(f'Xvfb exited before readiness: {server.returncode}')
            if select.select([read], [], [], .1)[0]:
                part = os.read(read, 64)
                if not part:
                    raise RuntimeError('Xvfb closed its readiness pipe.')
                data.extend(part)
                if b'\n' in data:
                    number = data.decode().strip()
                    if not number.isdecimal():
                        raise RuntimeError(f'Invalid Xvfb readiness response: {number!r}')
                    state.update(stage='ready', display=':'+number, pid=server.pid)
                    return server, ':'+number
        raise TimeoutError(f'Xvfb readiness exceeded {timeout}s (pid={server.pid}, exit={server.poll()}).')
    except BaseException as error:
        state.update(stage='failed', error=str(error), exit=server.poll() if server else None)
        if server:
            stop(server)
        raise
    finally:
        os.close(read)
        if write is not None:
            os.close(write)
        state['elapsed_seconds'] = round(time.monotonic()-began, 3)
        (output/'xvfb-status.json').write_text(json.dumps(state, indent=2)+'\n')


def stop(server):
    if server.poll() is None:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
