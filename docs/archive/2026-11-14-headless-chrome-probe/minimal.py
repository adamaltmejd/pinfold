#!/usr/bin/env python3
"""Check the Chromium image without the separate observation-tool layer."""
import json
import os
from pathlib import Path
import select
import subprocess

OUT = Path(__file__).resolve().parent
BIN = OUT.parents[2] / 'target/debug/pinfold'
W = Path((OUT / 'scratch-path').read_text())
ENV = {k: v for k, v in os.environ.items() if not k.startswith('PINFOLD_')}
ENV.update(XDG_CONFIG_HOME=str(W / 'config'), XDG_STATE_HOME=str(W / 'state'), PINFOLD_NO_UPDATE_CHECK='1')
ENV.pop('XDG_CACHE_HOME', None)
BOX = 'probe-chrome-minimal'
LOG = []

def scrub(value):
    return value.replace(str(Path.home()), '/Users/user').replace(str(W), '$W')

def run(args, data=None):
    result = subprocess.run(list(map(str, args)), env=ENV, input=data, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=45)
    text = f"$ {scrub(' '.join(map(str,args)))}\n{scrub(result.stdout)}\nexit={result.returncode}\n"
    LOG.append(text)
    print(text, flush=True)
    return result

spec = {'name': BOX, 'labels': {'org.pinfold.probe': 'chrome'}, 'profile': 'probe-chrome',
        'disks': [{'name': 'probe-chrome-minimal-home', 'guest': '/home/user', 'size': '1G'}, {'name': 'probe-chrome-minimal-work', 'guest': '/workspace', 'size': '1G'}],
        'env': {'HOME': '/home/user', 'PATH': '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin'},
        'egress': {'allow': ['api.github.com']}, 'memory': '2G'}
(OUT/'minimal-spec.json').write_text(json.dumps(spec, indent=2)+'\n')
owner = subprocess.Popen([str(BIN), 'box', 'up'], env=ENV, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
owner.stdin.write(json.dumps(spec)+'\n')
owner.stdin.flush()
ready = None
try:
    if not select.select([owner.stdout], [], [], 120)[0]:
        raise TimeoutError('no ready')
    line = owner.stdout.readline()
    LOG.append(scrub(line))
    ready = json.loads(line)
    if ready.get('event') != 'ready':
        raise RuntimeError(ready)
    run([BIN, 'box', 'exec', BOX, '--', 'sh', '-c', 'cat > /workspace/fixture.html'], (OUT/'fixture.html').read_text())
    run([BIN, 'box', 'exec', BOX, '--', 'chromium', '--headless', '--user-data-dir=/home/user/sandbox-on', '--dump-dom', 'about:blank'])
    run([BIN, 'box', 'exec', BOX, '--', 'chromium', '--headless', '--no-sandbox', '--user-data-dir=/home/user/minimal', '--dump-dom', 'https://api.github.com/'])
    run([BIN, 'box', 'exec', BOX, '--', 'chromium', '--headless', '--no-sandbox', '--user-data-dir=/home/user/minimal-render', '--screenshot=/workspace/minimal.png', 'file:///workspace/fixture.html'])
    run([BIN, 'box', 'exec', BOX, '--', 'sh', '-c', 'cat /etc/chromium.d/default-flags; cat /etc/chromium.d/dev-shm; du -shx /; find /usr /bin /sbin -xdev -perm /6000 -type f'], None)
finally:
    if ready and ready.get('event') == 'ready':
        run([BIN, 'box', 'down', BOX])
    owner.stdin.close()
    owner.wait(timeout=45)
    LOG.append(scrub(owner.stdout.read()+owner.stderr.read()))
    if ready and ready.get('event') == 'ready':
        (OUT/'minimal-egress.jsonl').write_text(scrub(Path(ready['egress_log']).read_text()))
        destination = W/f"export-minimal-{ready['run_id']}"
        result=run([BIN,'disk','export','probe-chrome-minimal-work',destination,'--image','localhost/pinfold/profile-probe-chrome:latest','--json'])
        if result.returncode:
            raise RuntimeError('export failed: keeping disks')
        import shutil
        shutil.copy2(destination/'minimal.png', OUT/'minimal.png')
        for disk in ['probe-chrome-minimal-home','probe-chrome-minimal-work']:
            run([BIN,'disk','rm',disk,'--json'])
    (OUT/'minimal.txt').write_text('\n'.join(LOG))
