#!/usr/bin/env python3
"""Verify only probe resources remain, inventory images, remove scratch."""
import json
import os
from pathlib import Path
import shutil
import subprocess

OUT = Path(__file__).resolve().parent
BIN = OUT.parents[2] / 'target/debug/pinfold'
W = Path((OUT/'scratch-path').read_text())
ENV = {k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
ENV.update(XDG_CONFIG_HOME=str(W/'config'), XDG_STATE_HOME=str(W/'state'), PINFOLD_NO_UPDATE_CHECK='1')
ENV.pop('XDG_CACHE_HOME', None)
lines=[]
for args in [[BIN,'box','list','--label','org.pinfold.probe=chrome'],[BIN,'disk','list','--json']]:
    result = subprocess.run(list(map(str,args)), env=ENV, capture_output=True, text=True, check=True)
    if result.stdout.strip():
        raise RuntimeError('probe resources remain; refusing scratch removal')
    lines.append('$ '+' '.join(map(str,args))+'\n'+result.stdout+result.stderr+'exit=0\n')
result=subprocess.run(['container','image','list','--format','json'],env=ENV,capture_output=True,text=True,check=True)
images=json.loads(result.stdout)
owned=[value for value in images if 'localhost/pinfold/profile-probe-chrome' in json.dumps(value)]
(OUT/'images-left.json').write_text(json.dumps(owned,indent=2)+'\n')
lines.append('$ container image list --format json\nFiltered to the probe-chrome repositories in images-left.json.\nexit=0\n')
if not W.name.startswith('pfc.') or W.parent != Path('/private/tmp'):
    raise RuntimeError('unexpected scratch path')
shutil.rmtree(W)
lines.append('Removed only the probe scratch root. Container service left running.\n')
text='\n'.join(lines).replace(str(Path.home()),'/Users/user').replace(str(W),'$W')
(OUT/'cleanup.txt').write_text(text)
print(text)
(OUT/'scratch-path').unlink()
