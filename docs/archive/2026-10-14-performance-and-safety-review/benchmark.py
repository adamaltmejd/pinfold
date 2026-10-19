import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import statistics
import subprocess
import tempfile
import time
import uuid

BINARY = Path('<checkout>/target/release/pinfold')
OUT = Path('/private/tmp/pinfold-review-benchmark.json')
ROOT = Path(tempfile.mkdtemp(prefix='pf-review-', dir='/private/tmp'))
ENV = {k: v for k, v in os.environ.items() if not k.startswith('PINFOLD_')}
ENV.update({f'XDG_{kind}_HOME': str(ROOT / kind.lower()) for kind in ['STATE', 'CACHE', 'CONFIG']})
state = ROOT / 'state' / 'pinfold'
state.mkdir(parents=True)
(state / 'maintenance').touch()
records = []
token = uuid.uuid4().hex[:10]

def run(args):
    t = time.perf_counter()
    p = subprocess.run(args, env=ENV, stdin=subprocess.DEVNULL, capture_output=True, timeout=30)
    elapsed = time.perf_counter() - t
    if p.returncode:
        raise RuntimeError(f'{args[:3]}: {p.returncode}: {p.stderr.decode(errors="replace")}')
    return elapsed, p.stdout

def inventory():
    return json.loads(run(['container', 'list', '--all', '--format', 'json'])[1])

def line(owner):
    with selectors.DefaultSelector() as sel:
        sel.register(owner.stdout, selectors.EVENT_READ)
        if not sel.select(45):
            raise RuntimeError('owner emitted no event within 45 seconds')
    raw = owner.stdout.readline()
    if not raw:
        raise RuntimeError(f'owner ended before event: {owner.stderr.read().decode(errors="replace")}')
    return json.loads(raw)

def save():
    OUT.write_text(json.dumps({'binary': str(BINARY), 'sha256': hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        'version': run([str(BINARY), '--version'])[1].decode().strip(), 'fixture_root': str(ROOT),
        'notes': 'Sequential warm/uncontrolled cache observations, not a speedup comparison. Maintenance excluded using a fresh private stamp. No harness/model, external network or image build. Fixture creation excluded.',
        'records': records}, indent=2))

try:
    existing = [x['id'] for x in inventory() if 'dev.pinfold.owner' in x['configuration'].get('labels', {})]
    if existing:
        raise RuntimeError(f'active Pinfold runtime entries: {existing}')
    for i in range(15):
        records.append({'phase': 'version', 'seconds': run([str(BINARY), '--version'])[0]})
    tree = ROOT / 'tree'
    (tree / '.git').mkdir(parents=True)
    (tree / '.git' / 'config').write_text('[core]\n bare = false\n')
    for d in range(100):
        directory = tree / str(d)
        directory.mkdir()
        for n in range(1000):
            (directory / str(n)).touch()
    for variant in ['empty', 'proxy', 'tree100k']:
        for i in range(5):
            name = f'review-{token}-{variant}-{i}'
            spec = {'name': name, 'image': 'pinfold/profile-default:latest', 'cpus': 2, 'memory': '1G',
                    'labels': {'dev.review.run': token}}
            if variant == 'proxy':
                spec['egress'] = {'allow': []}
            if variant == 'tree100k':
                spec['mounts'] = [{'host': str(tree), 'guest': '/workspace', 'readonly': False},
                                  {'host': str(tree / '.git'), 'guest': '/workspace/.git', 'readonly': True}]
            owner = subprocess.Popen([str(BINARY), 'box', 'up'], env=ENV, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            started = time.perf_counter()
            owner.stdin.write(json.dumps(spec).encode() + b'\n')
            owner.stdin.flush()
            try:
                ready = line(owner)
                elapsed = time.perf_counter() - started
                if ready.get('event') != 'ready':
                    raise RuntimeError(str(ready))
                records.append({'phase': 'ready', 'variant': variant, 'trial': i,
                                'seconds': elapsed, 'image_id': ready['image']['id']})
                if i == 1:
                    native = next(x for x in inventory() if x['id'] == name)
                    init = native['configuration']['initProcess']['executable']
                    for j in range(10):
                        pairs = [('pinfold_exec', [str(BINARY), 'box', 'exec', name, '--', '/bin/true']),
                                 ('native_exec_same_init', ['container', 'exec', '-i', name, init, 'init', 'exec', '--', '/bin/true']),
                                 ('pinfold_list', [str(BINARY), 'box', 'list', '--label', f'dev.review.run={token}']),
                                 ('native_list', ['container', 'list', '--all', '--format', 'json'])]
                        if j % 2:
                            pairs.reverse()
                        for phase, args in pairs:
                            records.append({'phase': phase, 'variant': variant, 'trial': j, 'seconds': run(args)[0]})
            finally:
                ended = time.perf_counter()
                owner.stdin.close()
                owner.wait(timeout=35)
                tail = owner.stdout.read().decode()
                err = owner.stderr.read().decode(errors='replace')
                records.append({'phase': 'down', 'variant': variant, 'trial': i,
                                'seconds': time.perf_counter()-ended, 'exit': owner.returncode, 'tail': tail, 'stderr': err})
                if owner.returncode != 0:
                    raise RuntimeError(f'owner failed: {owner.returncode}: {err}')
            save()
    leftovers = [x['id'] for x in inventory() if x['configuration'].get('labels', {}).get('dev.review.run') == token]
    if leftovers:
        raise RuntimeError(f'review boxes remain: {leftovers}')
    save()
    for phase in sorted({r['phase'] for r in records}):
        for variant in sorted({r.get('variant', '') for r in records if r['phase'] == phase}):
            vals = [r['seconds'] for r in records if r['phase'] == phase and r.get('variant', '') == variant]
            print(json.dumps({'phase': phase, 'variant': variant, 'n': len(vals),
                              'median_s': statistics.median(vals), 'min_s': min(vals), 'max_s': max(vals)}), flush=True)
finally:
    save()
    if not [x for x in inventory() if x['configuration'].get('labels', {}).get('dev.review.run') == token]:
        shutil.rmtree(ROOT)
