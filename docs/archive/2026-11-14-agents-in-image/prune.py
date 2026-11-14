#!/usr/bin/env python3
"""Remove explicit obsolete PR 2 Linux images under production catalog/lease locks."""
import datetime, fcntl, hashlib, json, os, pathlib, re, shutil, stat, sys
if sys.flags.optimize: raise SystemExit('run without Python optimization')

cache = pathlib.Path(os.environ.get('XDG_CACHE_HOME', pathlib.Path.home()/'.cache'))/'pinfold/images'
cache = cache.resolve(); host = pathlib.Path('/tmp')/f'pinfold-{os.getuid()}'
assert sys.platform == 'linux' and host.stat().st_uid == os.getuid()
assert stat.S_IMODE(host.stat().st_mode) == 0o700
locks = []
def lock(suffix):
    key = 'images:'+hashlib.sha256(os.fsencode(cache)).hexdigest()+':'+suffix
    path = host/(hashlib.sha256(key.encode()).hexdigest()+'.lock')
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    assert os.fstat(fd).st_uid == os.getuid() and stat.S_IMODE(os.fstat(fd).st_mode) == 0o600
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB); locks.append(fd)
lock('catalog')
path = cache/'catalog.json'; original = path.read_bytes(); data = json.loads(original)
retire = '--retire' in sys.argv
ids = set(sys.argv[1:]) - {'--retire'}; refs = data['references']; removed = {}
start = int(datetime.datetime(2026,11,14,20,3,tzinfo=datetime.timezone.utc).timestamp())*10**9
for identity in ids:
    assert re.fullmatch('sha256:[0-9a-f]{64}', identity)
    matches = [r for r,i in refs.items() if i == identity]
    assert matches and (retire or all(not r.endswith(':latest') for r in matches))
    profile = data['images'][identity]['labels']['dev.pinfold.profile']
    assert profile in ('default', 'documents', 'full', 'probe-pr2-failure', '')
    family = 'profile-'+profile
    if not profile:
        project = data['images'][identity]['labels']['dev.pinfold.project']
        assert project == 'project-d7f66ec20ade'; family = 'project-'+project
    assert not retire or profile in ('default', 'documents', 'probe-pr2-failure')
    assert all(r.startswith('localhost/pinfold/'+family+':') and (r.endswith(':latest') or int(r.rsplit(':',1)[1].split('-')[0],16) >= start) for r in matches)
    assert any(not r.endswith(':latest') for r in matches)
    lock(identity); directory = cache/identity[7:]
    assert not directory.is_symlink() and stat.S_IMODE(directory.stat().st_mode) == 0o700
    assert directory.stat().st_uid == os.getuid()
    fd = os.open(directory/'.pinfold-image', os.O_RDONLY | os.O_NOFOLLOW); marker = os.fstat(fd)
    assert stat.S_ISREG(marker.st_mode) and marker.st_uid == os.getuid() and stat.S_IMODE(marker.st_mode) == 0o600
    with os.fdopen(fd,'rb') as source: assert source.read() == ('pinfold-image-v1\n'+identity+'\n').encode()
    removed[identity] = matches
backup = pathlib.Path(os.environ['PROBE_ROOT'])/'catalog-before-prune.json'
fd = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
with os.fdopen(fd,'wb') as out: out.write(original); out.flush(); os.fsync(out.fileno())
data['references'] = {r:i for r,i in refs.items() if i not in ids}
data['images'] = {i:v for i,v in data['images'].items() if i not in ids}
staging = cache/('prune-'+str(os.getpid())+'.json')
fd = os.open(staging, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd,'w') as out: json.dump(data,out); out.flush(); os.fsync(out.fileno())
os.replace(staging,path)
fd = os.open(cache,os.O_RDONLY); os.fsync(fd); os.close(fd)
for identity in ids: shutil.rmtree(cache/identity[7:])
print('removed-obsolete-owned-profiles='+json.dumps(removed,sort_keys=True))
