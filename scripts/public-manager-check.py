#!/usr/bin/env python3
"""Launch the unpacked public source Manager, without installing anything."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from xvfb_support import start, stop

root = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1] if len(sys.argv)>1 else root/'target/public-manager').resolve()
output.mkdir(parents=True, exist_ok=True)
checks=[]
def check(name, passed):
    checks.append({'name':name,'passed':bool(passed)})
    (output/'results.json').write_text(json.dumps({'checks':len(checks),'passed':sum(c['passed'] for c in checks),'results':checks},indent=2)+'\n')
    assert passed,name

server, display = start(output)
try:
    with tempfile.TemporaryDirectory(prefix='mochi-public-source-') as directory:
        env={**os.environ,'DISPLAY':display,'GDK_BACKEND':'x11','WAYLAND_DISPLAY':'','GTK_A11Y':'none','GSK_RENDERER':'cairo','LANGUAGE':'en',
             'XDG_CONFIG_HOME':directory+'/config','XDG_DATA_HOME':directory+'/data','XDG_STATE_HOME':directory+'/state','XDG_RUNTIME_DIR':directory+'/runtime',
             'MOCHI_MANAGER_V2_NON_UNIQUE':'1'}
        probe="import json;from theme_store import ThemeStore;from mochi_paths import PUBLIC_SOURCE,INSTALLED; s=ThemeStore();print(json.dumps({'public':PUBLIC_SOURCE,'installed':INSTALLED,'theme':s.get(),'missing':len(s.missing_resources(s.get()))}))"
        data=json.loads(subprocess.check_output(['/usr/bin/python3','-c',probe],env={**env,'PYTHONPATH':str(root/'platform/linux')},text=True))
        check('Distribution marker selects source mode',data['public'] and not data['installed'])
        check('Welcome uses only original available resources',data['theme']['id']=='welcome' and data['theme']['characters']==['mochi-dot'] and data['theme']['bubble']=='default' and data['missing']==0)
        # Direct source entrypoint proves this is not the installed launcher.
        with (output/'manager.log').open('w') as log:
            process=subprocess.Popen(['/usr/bin/python3',str(root/'platform/linux/manager_v2.py')],env=env,cwd=root,stdout=log,stderr=log)
            try:
                from PIL import ImageGrab, ImageStat
                deadline=time.monotonic()+25
                while time.monotonic()<deadline:
                    if process.poll() is not None:raise AssertionError('Public source Manager exited')
                    info=subprocess.run(['xwininfo','-name','Mochi Manager'],env=env,capture_output=True,text=True)
                    if 'IsViewable' in info.stdout:
                        image=ImageGrab.grab(xdisplay=display)
                        if ImageStat.Stat(image.convert('L')).var[0]>100:
                            image.save(output/'public-source-manager.png');break
                    time.sleep(.1)
                else:raise AssertionError('No visible first frame')
                check('Uninstalled GTK4 source Manager renders a nonempty first frame',process.poll() is None)
            finally:
                if process.poll() is None:process.terminate();process.wait(timeout=10)
finally:stop(server)
print(json.dumps({'checks':len(checks),'passed':sum(c['passed'] for c in checks)}))
