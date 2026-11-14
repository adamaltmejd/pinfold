#!/usr/bin/env python3
"""Manual Linux evidence collector. Run only on the already leased trial VM."""
import datetime, json, os, pathlib, re, shutil
import subprocess, sys, time

D = pathlib.Path(__file__).resolve().parent
ROOT = D.parents[2]
P = ROOT / 'target/x86_64-unknown-linux-musl/debug/pinfold'
W = pathlib.Path('/tmp/pfl298')
W.mkdir(exist_ok=True)
ENV = {k: os.environ[k] for k in ('HOME', 'PATH', 'USER', 'LOGNAME') if k in os.environ}
ENV.update(XDG_CONFIG_HOME=str(W/'config'), XDG_STATE_HOME=str(W/'state'),
           PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
(W/'state/pinfold').mkdir(parents=True, exist_ok=True)
(W/'state/pinfold/maintenance').touch()
MODE = sys.argv[1]
LOG = W/(MODE+'.log')

def emit(s):
    s = str(s).replace(str(pathlib.Path.home()), '/home/user')
    with LOG.open('a') as f:
        f.write(s+'\n')
    print(s, flush=True)

def deadline():
    if datetime.datetime.now(datetime.timezone.utc) >= datetime.datetime(2026, 11, 15, 0, 28, tzinfo=datetime.timezone.utc):
        raise RuntimeError('lease safety cutoff reached')

def run(args, timeout=120, input=None):
    deadline()
    emit('$ '+' '.join(map(str, args)))
    start = time.monotonic()
    p = subprocess.run(list(map(str,args)), env=ENV, capture_output=True, text=True, timeout=timeout, input=input)
    emit(p.stdout+p.stderr)
    emit(f'[exit {p.returncode}; seconds {time.monotonic()-start:.3f}]')
    return p

def audit(path):
    rows = [json.loads(s) for s in pathlib.Path(path).read_text().splitlines()]
    counts = {}
    for row in rows:
        key = json.dumps({k:v for k,v in row.items() if k not in ('time','timestamp')}, sort_keys=True)
        counts[key] = counts.get(key,0)+1
    for key, count in sorted(counts.items()):
        emit(f'audit count={count} {key}')

if MODE in ('build-default','build-final','build-retry','build-diagnostic','build-metadata'):
    profile = W/'config/pinfold/profiles/probe-agents'
    profile.mkdir(parents=True,exist_ok=True)
    recipe = (ROOT/'docs/archive/2026-11-14-agents-image-updaters-logins/Containerfile'
              if MODE == 'build-default' else D/'Containerfile')
    text = recipe.read_text()
    if MODE == 'build-diagnostic':
        text += '''RUN cat /usr/local/share/probe-agents/install-time /usr/local/share/probe-agents/packages /usr/local/share/probe-agents/cache-before /usr/local/share/probe-agents/cache-after; \
    du -sk /usr/local/lib/node_modules/* /usr/local/bin/node; \
    find /usr/local/lib/node_modules -type f -size +30M -printf '%s %p\\n'; \
    du -sk /usr /root /opt /var; exit 1\n'''
    if MODE == 'build-metadata':
        text += 'RUN cat /usr/local/share/probe-agents/install-time /usr/local/share/probe-agents/packages /usr/local/share/probe-agents/cache-before /usr/local/share/probe-agents/cache-after; node --version; npm --version; exit 1\n'
    (profile/'Containerfile').write_text(text)
    labels = {'build-default':('default',), 'build-retry':('unchanged-after-failure',),
              'build-diagnostic':('diagnostic',), 'build-metadata':('metadata',)}
    for label in labels.get(MODE, ('cold','unchanged')):
        emit('== '+label)
        run(['df','-h','/'])
        if shutil.disk_usage('/').free < 12*1024**3:
            raise RuntimeError('under 12 GiB free')
        args = [P,'build','--profile','probe-agents']
        if MODE != 'build-default':
            args += ['--allow','nodejs.org']
        p = run(args,timeout=900)
        for path in re.findall(r'egress log (\S+)',p.stderr):
            audit(path)
        if p.returncode:
            break
    catalog = pathlib.Path.home()/'.cache/pinfold/images/catalog.json'
    if catalog.exists():
        data = json.loads(catalog.read_text())
        for ref, identity in data['references'].items():
            if '/profile-probe-agents:' in ref:
                emit('catalog '+json.dumps({'ref':ref,'id':identity,**data['images'][identity]}))
elif MODE == 'state':
    catalog = json.loads((pathlib.Path.home()/'.cache/pinfold/images/catalog.json').read_text())
    refs = {r:i for r,i in catalog['references'].items() if '/profile-probe-agents:' in r}
    emit('probe-agents refs '+json.dumps(refs))
    run([P,'box','list','--label','dev.pinfold.profile=probe-agents'])
    run([P,'disk','list','--json'])
    if refs:
        raise RuntimeError('image exists; refuse an unattended launch')
    spec = {'name':'probe-linux298-missing','image':'localhost/pinfold/profile-probe-agents:latest',
            'cpus':2,'memory':'1G','env':{'HOME':'/home/user'}}
    emit('spec '+json.dumps(spec))
    run([P,'box','up'], input=json.dumps(spec)+'\n')
    run(['df','-h','/'])
else:
    raise RuntimeError('unknown mode')
