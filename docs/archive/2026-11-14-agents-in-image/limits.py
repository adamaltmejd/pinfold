#!/usr/bin/env python3
"""Manual Linux bounds evidence. Run modes separately; cache runs last."""
import datetime, fcntl, hashlib, json, os, pathlib, re, select, shutil
import subprocess, sys, time

REPO = pathlib.Path(__file__).resolve().parents[3]
P = pathlib.Path(os.environ.get('PINFOLD_BIN', REPO/'target/x86_64-unknown-linux-musl/debug/pinfold'))
W = pathlib.Path(os.environ['PROBE_ROOT']).resolve()
if sys.platform != 'linux' or not str(W).startswith('/tmp/pfi.') or len(str(W)) > 35:
    raise RuntimeError('use Linux and a short dedicated /tmp/pfi. scratch root')
W.mkdir(exist_ok=True)
ENV = {k: os.environ[k] for k in ('HOME', 'PATH', 'USER', 'LOGNAME', 'XDG_CACHE_HOME') if k in os.environ}
ENV.update(XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'),
           PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
(W/'s/pinfold').mkdir(parents=True, exist_ok=True)
(W/'s/pinfold/maintenance').touch()
CACHE = pathlib.Path(ENV.get('XDG_CACHE_HOME', pathlib.Path.home()/'.cache'))/'pinfold'
LOG = W/'limits.log'
ROOT_MIB = int(os.environ.get('PINFOLD_ROOT_MIB', '2048'))
OCI_MIB = int(os.environ.get('PINFOLD_OCI_MIB', '1024'))
BASE_MIB = int(os.environ.get('PINFOLD_BASE_MIB', '1024'))
OWN = W/'limit-images.json'
owned = json.loads(OWN.read_text()) if OWN.exists() else []
TAG = 'localhost/pinfold/image-'

def emit(value):
    text = str(value).replace(str(REPO), '/home/user/pinfold').replace(str(pathlib.Path.home()), '/home/user')
    print(text, flush=True)
    with LOG.open('a') as out: out.write(text+'\n')

def run(args, timeout=180):
    if datetime.datetime.now(datetime.timezone.utc) >= datetime.datetime(2026, 11, 15, 0, 28, tzinfo=datetime.timezone.utc):
        raise RuntimeError('lease safety cutoff reached')
    emit('$ '+' '.join(map(str, args)))
    start = time.monotonic()
    result = subprocess.run(list(map(str, args)), env=ENV, capture_output=True, text=True, timeout=timeout)
    emit((result.stdout+result.stderr)[-9000:])
    emit(f'exit={result.returncode} seconds={time.monotonic()-start:.3f}')
    return result

def catalog():
    path = CACHE/'images/catalog.json'
    return json.loads(path.read_text()) if path.exists() else {'references': {}, 'images': {}}

def refs(name):
    return {r:i for r,i in catalog()['references'].items() if r.startswith(TAG+name+':')}

def build(name, recipe, reason=None):
    if name not in owned:
        if refs(name): raise RuntimeError('image exists without collector ownership')
        owned.append(name); OWN.write_text(json.dumps(owned))
    context = W/'limits-context'; context.mkdir(exist_ok=True)
    path = W/'Limitfile'; path.write_text(recipe)
    before = refs(name)
    run(['df', '-h', '/', '/tmp']); emit('free-bytes='+str(shutil.disk_usage('/').free))
    result = run([P, 'image', 'build', name, '--containerfile', path, '--context', context], 1200)
    after = refs(name)
    emit('refs-before='+json.dumps(before)+' refs-after='+json.dumps(after))
    if reason:
        if result.returncode == 0 or not any(s in result.stdout+result.stderr for s in reason) or after != before:
            raise RuntimeError('intended limit/no-publication outcome not established')
    else:
        if result.returncode: raise RuntimeError('smaller control did not build')
        identity = json.loads(result.stdout)['id']; image = catalog()['images'][identity]
        layout = CACHE/'images'/identity[7:]/'oci'
        payload = (layout/'index.json').stat().st_size+sum(p.stat().st_size for p in (layout/'blobs/sha256').iterdir())
        emit(f'control id={identity} root_bytes={image["root_bytes"]} oci_bytes={payload}')
        launch(TAG+name+':latest')

def launch(image):
    name = 'probe-pr2-limit-'+W.name[4:]
    spec = {'name': name, 'image': image, 'cpus': 2, 'memory': '1G', 'env': {'HOME': '/tmp'}}
    emit('spec='+json.dumps(spec))
    err = (W/'limit-box.stderr').open('w')
    owner = subprocess.Popen([str(P), 'box', 'up'], env=ENV, stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=err, text=True)
    try:
        owner.stdin.write(json.dumps(spec)+'\n'); owner.stdin.flush()
        if not select.select([owner.stdout], [], [], 180)[0]: raise RuntimeError('box readiness deadline')
        ready = owner.stdout.readline(); emit(ready)
        if json.loads(ready).get('event') != 'ready': raise RuntimeError('box did not start')
        result = run([P, 'box', 'exec', name, '--', 'sh', '-c', 'test -r /etc/os-release; echo launch-ok'])
        if result.returncode or 'launch-ok' not in result.stdout: raise RuntimeError('launch control failed')
    finally:
        owner.stdin.close()
        try: owner.wait(timeout=90)
        except subprocess.TimeoutExpired:
            run([P, 'box', 'down', name]); owner.wait(timeout=90)
        emit(owner.stdout.read()); err.close(); emit((W/'limit-box.stderr').read_text())

def payload(count, random=False):
    return ('FROM debian:trixie-slim\nRUN dd if=/dev/'+('urandom' if random else 'zero')+
            f' of=/payload bs=1M count={count} status=none\n')

mode = sys.argv[1]
if datetime.datetime.now(datetime.timezone.utc) >= datetime.datetime(2026, 11, 15, 0, 28, tzinfo=datetime.timezone.utc):
    raise RuntimeError('lease safety cutoff reached')
emit(f'mode={mode} root_mib={ROOT_MIB} oci_mib={OCI_MIB} bases_mib={BASE_MIB}')
if mode in ('root', 'oci'):
    name = 'probe-pr2-limit-'+mode
    control = payload(512 if mode == 'root' else 640, mode == 'oci')
    build(name, control)
    reason = ['resource: converted root size'] if mode == 'root' else ['resource: image output file', 'image-output: payload limit']
    if mode == 'root':
        oversized = payload(ROOT_MIB+1)
    else:
        count = OCI_MIB//2+64
        oversized = payload(count, True)+f'RUN dd if=/dev/urandom of=/payload bs=1M count={count} status=none\n'
    build(name, oversized, reason)
    build(name, control)
elif mode == 'bases':
    a, b, derived = ['probe-pr2-limit-'+suffix for suffix in ('base-a', 'base-b', 'derived')]
    common = payload(BASE_MIB//2+64, True)
    build(a, common+'LABEL probe.base=a\n'); build(b, common+'LABEL probe.base=b\n')
    control = f'FROM {TAG+a}:latest\nRUN test -s /payload\n'
    build(derived, control)
    build(derived, control+f'COPY --from={TAG+b}:latest /payload /second\n', ['resource: build input size'])
    build(derived, control)
elif mode == 'remove-images':
    for name in owned:
        if run([P, 'image', 'rm', name]).returncode: raise RuntimeError('owned image removal failed')
    OWN.write_text('[]')
elif mode == 'cache':
    directory = pathlib.Path(os.environ['PINFOLD_BUILDER_CACHE']).resolve()
    if directory.parent != (CACHE/'builder').resolve() or not re.fullmatch('[0-9a-f]{64}', directory.name):
        raise RuntimeError('supply the exact versioned builder cache path used by this run')
    if (directory/'job').exists(): raise RuntimeError('builder job is present')
    key = 'box:builder-'+hashlib.sha256(os.fsencode(CACHE.resolve())).hexdigest()[:32]
    lock = pathlib.Path('/tmp')/f'pinfold-{os.getuid()}'/(hashlib.sha256(key.encode()).hexdigest()+'.lock')
    fd = os.open(lock, os.O_RDWR | os.O_NOFOLLOW); fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    try:
        image = os.environ.get('PINFOLD_LIMIT_IMAGE', 'localhost/pinfold/profile-full:latest')
        data = catalog(); identity = data['references'][image]
        if data['images'][identity]['root_bytes'] <= 512*1024**2: raise RuntimeError('image is below old limit')
        before = data['references']; emit('removing only idle builder cache '+str(directory)); shutil.rmtree(directory)
        if catalog()['references'] != before: raise RuntimeError('image refs changed')
        launch(image)
    finally: os.close(fd)
else:
    raise RuntimeError('choose root, oci, bases, remove-images, or cache; cache runs last')
