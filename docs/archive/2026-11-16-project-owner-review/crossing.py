#!/usr/bin/env python3
"""Manual review cases: crossing.py BIN IMAGE; fifty iterations, no retries."""
import hashlib, json, os, pathlib, select, subprocess as sp, sys, tempfile
B = str(pathlib.Path(sys.argv[1]).resolve()); IMAGE = sys.argv[2]
W = pathlib.Path(tempfile.mkdtemp(prefix='pfr-cross.', dir='/tmp')).resolve()
P, Q, INTO = W/'p', W/'q', W/'into'
HOST_HOME = str(pathlib.Path.home()); CACHE = os.environ.get('XDG_CACHE_HOME', str(pathlib.Path(HOST_HOME)/'.cache'))
E = dict(PATH=os.environ.get('PATH', '/usr/bin:/bin'), HOME=HOST_HOME,
         XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'), XDG_CACHE_HOME=CACHE,
         CODEX_HOME=str(W/'codex'), PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G',
         PINFOLD_ENV_REVIEW='fixture-forwarded', PFO_SHOULD_DROP='fixture-unrelated',
         EDITOR='true', VISUAL='true', GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch()
H = pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'
children = []; failures = []; cleanup_ok = True

def emit(value):
    text = str(value).replace(str(W), '$W').replace(B, '$BIN')
    print(text.replace(HOST_HOME, '/home/user' if sys.platform=='linux' else '/Users/user'), flush=True)

def invoke(args, cwd=P, env=None):
    return sp.run([B, *args], cwd=cwd, env=env or E, input=b'', capture_output=True, timeout=180)

def run(args, cwd=P, env=None, code=0):
    r = invoke(args, cwd, env)
    assert r.returncode == code, (args, r.returncode, r.stderr[:300])
    return r

def git(p, *args):
    return sp.run(['git', '-C', str(p), *args], env=E, check=True, capture_output=True).stdout.strip()

def ident(p): return p.name+'-'+hashlib.sha256(os.fsencode(p)).hexdigest()[:12]
def locator(p): return H/(hashlib.sha256(b'project-owner:'+os.fsencode(p)).hexdigest()+'.project')
def loc(p): return json.loads(locator(p).read_text())
def same(p, observed): assert loc(p) == observed, 'owner generation changed'

def witness_line():
    assert select.select([witness.stdout], [], [], 180)[0], 'witness marker absent'
    result = witness.stdout.readline()
    assert result, 'witness client ended before marker'
    return result

def ping():
    assert witness.poll() is None, 'witness client ended during crossing'
    witness.stdin.write(b'ping\n'); witness.stdin.flush()
    assert witness_line() == b'WITNESS-PONG\n'
    same(Q, q_owner)

for p in (P, Q, INTO):
    p.mkdir(); git(p, 'init', '-q', '-b', 'review')
for p in (P, Q):
    (p/'tracked').write_text('FIXTURE\n'); git(p, 'add', 'tracked')
    git(p, '-c', 'user.name=user', '-c', 'user.email=user@pinfold.invalid', 'commit', '-qm', 'fixture')
    run(['config', 'edit'], cwd=p)
    c = W/'c/pinfold/projects'/ident(p)/'pinfold.toml'
    c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
try:
    run(['start'], cwd=Q); q_owner = loc(Q)
    witness = sp.Popen([B, '--', 'sh', '-c', 'echo WITNESS-READY; while IFS= read -r x; do case "$x" in ping) echo WITNESS-PONG;; done) echo WITNESS-DONE; break;; esac; done'],
                       cwd=Q, env=E, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    children.append(witness); assert witness_line() == b'WITNESS-READY\n'
    run(['start']); p_owner = loc(P); ping()
    for index in range(50):
        for verb in ('push', 'fetch'):
            try:
                r = invoke([verb]); event = json.loads(r.stdout) if r.stdout else {}
                good = r.returncode == 0 and event.get('event') == ('pushed' if verb=='push' else 'fetched') and event.get('box') == p_owner['name']
                if not good:
                    failures.append((index+1, verb, r.returncode))
                    emit(f'crossing-failure iteration={index+1} verb={verb} exit={r.returncode} stderr={r.stderr[:250]!r}')
            except (sp.TimeoutExpired, json.JSONDecodeError) as error:
                failures.append((index+1, verb, type(error).__name__))
                emit(f'crossing-failure iteration={index+1} verb={verb} failure={type(error).__name__}')
        same(P, p_owner); ping()
    emit(f'crossing iterations=50 calls=100 failures={len(failures)} second-box=50-PONG same-generations=True retries=0')
    # Equivalent spellings of HOME must not change roots or the frozen startup policy.
    alias = W/'home-alias'; alias.symlink_to(HOST_HOME, target_is_directory=True)
    for label, home in [('trailing-slash', HOST_HOME+'/'), ('symlink-equivalent', str(alias))]:
        r = run(['--', 'printf', 'HOME-CONTROL'], env=dict(E, HOME=home))
        assert r.stdout == b'HOME-CONTROL'; same(P, p_owner)
        emit(f'HOME {label}: exit=0 marker=HOME-CONTROL same-generation=True')
    different = W/'different-home'; different.mkdir()
    r = run(['--', 'printf', 'MUST-NOT-RUN'], env=dict(E, HOME=str(different)), code=125)
    assert not r.stdout and b'config-changed' in r.stderr and b'roots.home' in r.stderr
    assert run(['--', 'printf', 'HOME-RESTORED']).stdout == b'HOME-RESTORED'; same(P, p_owner)
    emit('HOME different-root: exit=125/config-changed roots.home marker=absent; restored exit=0 same-generation=True')
    # Both public box fetch and project fetch use the same transfer implementation.
    args = ['box', 'fetch', p_owner['name'], '--repo', '/fixture-missing-repository', '--into', str(INTO)]
    refused = run(args, code=1); event = json.loads(refused.stdout)
    assert event.get('event') == 'refused' and event.get('reason') == 'spec'
    guest_head = run(['--', 'git', 'rev-parse', 'HEAD']).stdout.strip()
    branch = run(['--', 'git', 'symbolic-ref', '--short', 'HEAD']).stdout.strip().decode()
    args[args.index('/fixture-missing-repository')] = str(P)
    fetched = json.loads(run(args).stdout); remote_ref = f'refs/remotes/{p_owner["name"]}/{branch}'
    assert fetched.get('event') == 'fetched' and fetched['refs'][remote_ref].encode() == guest_head
    assert git(INTO, 'rev-parse', remote_ref) == guest_head; same(P, p_owner); ping()
    emit('public box fetch: missing-repository exit=1/spec; corrected same-box exit=0/fetched independent-guest-HEAD=host-ref')
    if sys.platform == 'linux':
        listed = [json.loads(x) for x in run(['box', 'list', '--label', 'dev.pinfold.owner']).stdout.splitlines()]
        owner = next(x['owner'] for x in listed if x['name'] == p_owner['name'] and x['labels']['dev.pinfold.generation'] == p_owner['generation'])
        proc = pathlib.Path('/proc')/str(owner)
        assert proc.joinpath('cwd').resolve() == pathlib.Path('/'), 'owner cwd pins project'
        names = {entry.split(b'=', 1)[0].decode() for entry in proc.joinpath('environ').read_bytes().split(b'\0') if entry}
        allowed = {'PATH', 'HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_CACHE_HOME', 'CODEX_HOME',
                   'PINFOLD_PROFILE', 'PINFOLD_CPUS', 'PINFOLD_MEMORY', 'PINFOLD_ALLOW', 'PINFOLD_ROUTES', 'PINFOLD_ENV_REVIEW'}
        required = {'PATH', 'HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_CACHE_HOME', 'CODEX_HOME',
                    'PINFOLD_CPUS', 'PINFOLD_MEMORY', 'PINFOLD_ENV_REVIEW'}
        assert required <= names <= allowed, 'owner environment differs from required startup inputs'
        assert 'PFO_SHOULD_DROP' not in names, 'owner kept unrelated launching environment'
        emit('owner hygiene: cwd=/ unrelated-fixture-env=absent retained-names='+','.join(sorted(names)))
    else: emit('INSPECTED owner environment/cwd on Mac; Linux /proc case exercises the common launch path')
    witness.stdin.write(b'done\n'); witness.stdin.flush(); assert witness_line() == b'WITNESS-DONE\n'
    witness.wait(timeout=180); out, err = witness.communicate(timeout=30)
    assert witness.returncode == 0 and not out and not err, 'witness completion changed'
    same(Q, q_owner); emit('second-box command: completed=0 full-WITNESS-DONE kept-generation=unchanged')
    assert not failures, 'crossing failures recorded above'
finally:
    for child in children:
        if child.poll() is None:
            child.terminate(); child.wait(timeout=180); child.communicate(timeout=30)
    for p in (P, Q):
        try:
            for args in (['stop'], ['disk', 'rm', 'pi-'+ident(p)+'-home'], ['disk', 'rm', 'pi-'+ident(p)+'-project']):
                r = invoke(args, cwd=p); cleanup_ok &= r.returncode == 0
                if r.returncode: emit(f'cleanup failure command={args[0]} exit={r.returncode} stderr={r.stderr[:250]!r}')
        except Exception as error:
            cleanup_ok = False; emit('cleanup failure='+type(error).__name__)
    emit(f'cleanup own-boxes/disks={cleanup_ok} fixture=$W'); assert cleanup_ok
