#!/usr/bin/env python3
"""Scratch real-update ABBA; reuses the repository's unmodified HTTPS fixture."""
import hashlib, json, os, platform, select, shutil, signal, subprocess, sys, tempfile, time
from pathlib import Path

if len(sys.argv) != 4:
    raise SystemExit('usage: update-abba.py A B OUTPUT')
a, b, out = map(lambda value: Path(value).resolve(), sys.argv[1:])
fixture = Path(__file__).parent / 'release_proxy.py'
if not fixture.is_file():
    raise SystemExit('Place the exact repository release_proxy.py beside this wrapper')
out.mkdir(parents=True, exist_ok=True)
if any(out.iterdir()):
    raise SystemExit('OUTPUT must be empty')
root = None
proxy_log = None
inputs = {name: path.read_bytes() for name, path in [('A', a), ('B', b)]}
if a == b or inputs['A'] == inputs['B']:
    raise SystemExit('A and B must be distinct real binaries')
input_metadata = {v: dict(size=len(inputs[v]), mode=path.stat().st_mode & 0o7777,
                         sha256=hashlib.sha256(inputs[v]).hexdigest())
                  for v, path in [('A', a), ('B', b)]}
expected = input_metadata['A']['sha256']
rows = [dict(order=i+1, variant=v, status='not-run') for i, v in enumerate('ABBA')]
proxy = None

def save():
    temporary = out / 'results.tmp'
    temporary.write_text(json.dumps(rows, indent=2)+'\n')
    temporary.replace(out / 'results.json')

def stopped(signum, _frame):
    raise KeyboardInterrupt(f'signal {signum}')
signal.signal(signal.SIGTERM, stopped)

def run(argv, env, stem, timeout=120):
    child = None
    try:
        with (out/(stem+'.stdout')).open('wb') as stdout, (out/(stem+'.stderr')).open('wb') as stderr:
            child = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL,
                                     stdout=stdout, stderr=stderr, start_new_session=True)
            code = child.wait(timeout=timeout)
            if code:
                raise RuntimeError(f'{stem} exited {code}')
    finally:
        if child is not None and child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=10)

base_env = {k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
for key in ('https_proxy', 'NO_PROXY', 'no_proxy'):
    base_env.pop(key, None)
try:
    save()
    root = Path(tempfile.mkdtemp(prefix='pf-update-abba-', dir=os.environ['RUNNER_TEMP']))
    (root/'release').write_bytes(inputs['A'])
    run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1',
         '-subj','/CN=api.github.com','-addext','subjectAltName=DNS:api.github.com,DNS:github.com',
         '-keyout',str(root/'key.pem'),'-out',str(root/'cert.pem')], base_env, 'certificate', 30)
    asset = f'pinfold-999.0.0-{platform.machine()}-unknown-linux-musl'
    proxy_log = (out/'proxy.stderr').open('wb')
    proxy = subprocess.Popen([sys.executable,str(fixture),str(root),asset],
                             stdout=subprocess.PIPE,stderr=proxy_log,start_new_session=True)
    deadline, readiness = time.monotonic() + 15, bytearray()
    while b'\n' not in readiness:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([proxy.stdout], [], [], max(remaining, 0))[0]:
            raise TimeoutError('fixture readiness deadline')
        chunk = os.read(proxy.stdout.fileno(), 32-len(readiness))
        if not chunk:
            raise RuntimeError('fixture readiness ended before port')
        readiness.extend(chunk)
        if len(readiness) == 32 and b'\n' not in readiness:
            raise RuntimeError('fixture readiness exceeds 32 bytes')
    if not readiness.endswith(b'\n') or not readiness[:-1].isdigit():
        raise RuntimeError('invalid fixture readiness')
    port = int(readiness[:-1])
    if not 0 < port < 65536:
        raise RuntimeError('invalid fixture port')
    base_env.update(HTTPS_PROXY=f'http://127.0.0.1:{port}', CURL_CA_BUNDLE=str(root/'cert.pem'))
    expected_version = None
    for row in rows:
        stem = str(row['order'])+'-'+row['variant']
        row['status'] = 'attempting'; save()
        work = root/stem
        installed = work/'bin/pinfold'
        installed.parent.mkdir(parents=True)
        installed.write_bytes(inputs[row['variant']]); installed.chmod(0o755)
        env = base_env.copy()
        for key, directory in [('XDG_STATE_HOME','state'),('XDG_CACHE_HOME','cache'),('XDG_CONFIG_HOME','config')]:
            path = work/directory; path.mkdir(); env[key] = str(path)
        run([str(installed),'--version'],env,stem+'-before-version',30)
        version = (out/(stem+'-before-version.stdout')).read_bytes()
        expected_version = version if expected_version is None else expected_version
        assert version == expected_version, 'A/B versions differ'
        before = installed.stat().st_ino
        timing = out/(stem+'.time')
        try:
            run(['/usr/bin/time','-f','%e %U %S %M %x','-o',str(timing),str(installed),'update'],env,stem)
        finally:
            try:
                with timing.open() as stream:
                    metrics = stream.read(1025)
                if len(metrics) > 1024:
                    raise ValueError('timing output exceeds bound')
                wall,user,system,rss,code = metrics.splitlines()[-1].split()
                row.update(wall_seconds=float(wall), user_seconds=float(user),
                           system_seconds=float(system), max_rss_kib=int(rss), exit=int(code))
            except (OSError, ValueError, IndexError) as error:
                row['timing_error'] = type(error).__name__
            save()
        if 'timing_error' in row:
            raise RuntimeError('timing unavailable; inspect diagnostic files')
        row.update(inode_before=before, inode_after=installed.stat().st_ino)
        assert row['inode_after'] != before, 'update did not replace inode'
        assert installed.read_bytes() == inputs['A'], 'installed bytes differ from payload'
        assert hashlib.sha256(installed.read_bytes()).hexdigest() == expected
        run([str(installed),'--version'],env,stem+'-version',30)
        assert (out/(stem+'-version.stdout')).read_bytes() == expected_version
        row.update(status='success', payload_sha256=expected); save()
except BaseException as error:
    for row in rows:
        if row['status'] == 'attempting':
            row.update(status='failed', error=str(error))
    save()
    raise
finally:
    cleanup_error = None
    try:
        if proxy is not None:
            if proxy.poll() is None:
                os.killpg(proxy.pid, signal.SIGTERM)
            try:
                proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proxy.pid, signal.SIGKILL); proxy.wait(timeout=10)
    except BaseException as error:
        cleanup_error = type(error).__name__
    finally:
        if proxy_log is not None:
            proxy_log.close()
        try:
            try:
                unchanged = a.read_bytes() == inputs['A'] and b.read_bytes() == inputs['B']
            except OSError:
                unchanged = False
            (out/'provenance.json').write_text(json.dumps(dict(inputs_unchanged=unchanged,
                inputs=input_metadata, cleanup_error=cleanup_error,
                fixture_sha256=hashlib.sha256(fixture.read_bytes()).hexdigest(),
                scope='diagnostic only'))+'\n')
        finally:
            if root is not None:
                shutil.rmtree(root)
    if cleanup_error:
        raise RuntimeError('fixture process cleanup failed')
    assert unchanged, 'input binary changed'
