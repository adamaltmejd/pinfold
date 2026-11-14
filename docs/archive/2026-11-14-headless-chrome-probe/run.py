#!/usr/bin/env python3
"""Run only the probe's boxes, export its work disk, remove its disks."""
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2].parent
OUT = Path(__file__).resolve().parent
BIN = ROOT / 'target/debug/pinfold'
W = Path((OUT / 'scratch-path').read_text())
ENV = {k: v for k, v in os.environ.items() if not k.startswith('PINFOLD_')}
ENV.update(XDG_CONFIG_HOME=str(W / 'config'), XDG_STATE_HOME=str(W / 'state'), PINFOLD_NO_UPDATE_CHECK='1')
ENV.pop('XDG_CACHE_HOME', None)
BOX = 'probe-chrome-observe'

def scrub(value):
    return value.replace(str(Path.home()), '/Users/user').replace(str(W), '$W').replace(str(W).removeprefix('/private'), '$W')

def run(args, name, data=None, timeout=180):
    start = time.monotonic()
    result = subprocess.run(list(map(str, args)), env=ENV, input=data, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout)
    output = f"$ {scrub(' '.join(map(str, args)))}\n{scrub(result.stdout)}\nexit={result.returncode}; elapsed_seconds={time.monotonic()-start:.3f}\n"
    (OUT / name).write_text(output)
    print(output[:(1000 if '-apt.txt' in name else 16000)], flush=True)
    return result

spec = {'name': BOX, 'labels': {'org.pinfold.probe': 'chrome'}, 'profile': 'probe-chrome', 'image': 'localhost/pinfold/profile-probe-chrome-tools:latest',
        'disks': [{'name': 'probe-chrome-home', 'guest': '/home/user', 'size': '1G'}, {'name': 'probe-chrome-work', 'guest': '/workspace', 'size': '1G'}],
        'env': {'HOME': '/home/user', 'PATH': '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin'},
        'egress': {'allow': ['api.github.com']}, 'cpus': 4, 'memory': '2G'}
(OUT / 'spec.json').write_text(json.dumps(spec, indent=2) + '\n')
owner = subprocess.Popen([str(BIN), 'box', 'up'], env=ENV, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
owner.stdin.write(json.dumps(spec) + '\n')
owner.stdin.flush()
ready = None
try:
    if not select.select([owner.stdout], [], [], 120)[0]:
        raise TimeoutError('no box ready')
    line = owner.stdout.readline()
    (OUT / 'up.jsonl').write_text(scrub(line))
    print(scrub(line), flush=True)
    ready = json.loads(line)
    if ready.get('event') != 'ready':
        raise RuntimeError(ready)
    for src in ['guest.py', 'fixture.html']:
        run([BIN, 'box', 'exec', BOX, '--', 'sh', '-c', f'cat > /workspace/{src}'], f'copy-{src}.txt', (OUT / src).read_text())
    run([BIN, 'box', 'exec', BOX, '--', 'sh', '-c', 'chromium --version; df -h /dev/shm; id; du -sh /usr/lib/chromium; find / -xdev -perm /6000 -type f 2>/dev/null'], 'guest-facts.txt')
    for name in ['probe-chrome', 'probe-chrome-tools']:
        run([BIN, 'box', 'exec', BOX, '--', 'cat', f'/opt/{name}/build.log'], f'{name}-apt.txt')
    for case in sys.argv[1:] or ['env-only', 'explicit-proxy']:
        result = run([BIN, 'box', 'exec', BOX, '--', 'python3', '/workspace/guest.py', case], f'{case}.jsonl.txt')
        if result.returncode:
            print(f'{case} failed; preserved output', flush=True)
        (OUT / f'{case}-egress.jsonl').write_text(scrub(Path(ready['egress_log']).read_text()))
    run([BIN, 'box', 'stat', BOX], 'stat.json.txt')
finally:
    if ready and ready.get('event') == 'ready':
        run([BIN, 'box', 'down', BOX], 'down.txt')
    owner.stdin.close()
    owner.wait(timeout=45)
    with (OUT / 'up.jsonl').open('a') as log:
        log.write(scrub(owner.stdout.read()))
    (OUT / 'owner-stderr.txt').write_text(scrub(owner.stderr.read()))
    if ready and ready.get('event') == 'ready':
        (OUT / 'egress.jsonl').write_text(scrub(Path(ready['egress_log']).read_text()))
        destination = W / f"export-{ready['run_id']}"
        exported = run([BIN, 'disk', 'export', 'probe-chrome-work', destination, '--image', 'localhost/pinfold/profile-probe-chrome-tools:latest', '--json'], 'export.txt')
        if exported.returncode:
            raise RuntimeError('export failed; keeping disks for recovery')
        if destination.is_dir():
            import shutil
            target = OUT / 'work'
            shutil.copytree(destination, target)
            for path in target.iterdir():
                if path.suffix in ['.txt', '.json', '.py', '.html']:
                    path.write_text(scrub(path.read_text()))
        for name in ['probe-chrome-home', 'probe-chrome-work']:
            run([BIN, 'disk', 'rm', name, '--json'], f'remove-{name}.txt')
        run([BIN, 'disk', 'list', '--json'], 'disks-after.txt')
        run([BIN, 'box', 'list', '--label', 'org.pinfold.probe=chrome'], 'boxes-after.txt')
