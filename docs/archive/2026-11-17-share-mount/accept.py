"""Run the #279 Apple acceptance cases through the real public CLI."""
import hashlib, json, os, pathlib, selectors, shlex, shutil, subprocess, tempfile, time

BIN = pathlib.Path('target/debug/pinfold').resolve()
BASE = 'localhost/pinfold/profile-default'
IMAGE = os.environ.get('PF279_IMAGE', BASE)
W = pathlib.Path(tempfile.mkdtemp(prefix='pf279.', dir='/tmp')).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update(XDG_CONFIG_HOME=str(W / 'config'), XDG_STATE_HOME=str(W / 'state'), PINFOLD_NO_UPDATE_CHECK='1')
stamp = W / 'state/pinfold/maintenance'; stamp.parent.mkdir(parents=True); stamp.touch()
git_env = dict(env, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null', GIT_CONFIG_NOSYSTEM='1')
owners, disks = {}, []
serial = 0

def clean(value):
    text = str(value).replace(str(W), '$W').replace(str(W).removeprefix('/private'), '$W')
    return text.replace(str(pathlib.Path.home()), '/Users/user')

def say(value):
    print(clean(value), flush=True)

def run(args, expected=0, label=None, **kwargs):
    p = subprocess.run([str(x) for x in args], env=kwargs.pop('env', env), text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90, **kwargs)
    lines = p.stdout.strip().splitlines()
    say(f'{label or shlex.join([str(x) for x in args])}: rc={p.returncode} ' + ' | '.join(lines[:6]))
    if expected is not None:
        assert p.returncode == expected, clean(p.stdout)
    return p

def pf(*args, **kwargs):
    return run([BIN, *args], **kwargs)

def git(*args):
    return run(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'user.name=user', '-c',
                'user.email=user@example.invalid', '-C', share, *args], env=git_env)

def mount(host, guest='/project', kind='share'):
    return dict(host=str(host), guest=guest, kind=kind)

def spec(mounts, **extra):
    global serial
    serial += 1
    return dict(name=f'probe-share279-{os.getpid()}-{serial}', image=BASE, mounts=mounts,
                cpus=2, memory='1G', env={'HOME': '/tmp'}, **extra)

def line_until(p, predicate, timeout=180):
    deadline = time.monotonic() + timeout
    with selectors.DefaultSelector() as selector:
        selector.register(p.stdout, selectors.EVENT_READ)
        while time.monotonic() < deadline:
            assert selector.select(max(0, deadline - time.monotonic())), 'readiness timeout'
            line = p.stdout.readline()
            assert line, f'process ended before readiness: rc={p.poll()}'
            text = line.rstrip()
            if predicate(text):
                return text
            say('startup: ' + text)
    raise AssertionError('readiness timeout')

def start(s):
    p = subprocess.Popen([str(BIN), 'box', 'up'], env=env, text=True,
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    owners[s['name']] = p
    p.stdin.write(json.dumps(s) + '\n'); p.stdin.flush()
    text = line_until(p, lambda x: x.startswith('{'))
    event = json.loads(text)
    say(f'box up {s["name"]}: {text}')
    assert event['event'] == 'ready', text
    return s['name'], event['run_id']

def stop(name):
    pf('box', 'down', name)
    p = owners.pop(name); p.stdin.close(); p.wait(timeout=90)
    say(f'owner {name}: rc={p.returncode} ' + p.stdout.read().strip())
    pf('box', 'stat', name, expected=3)

def refuse(label, mounts, **extra):
    s = spec(mounts, **extra)
    p = pf('box', 'up', input=json.dumps(s), expected=1, label=label)
    event = json.loads(p.stdout.strip().splitlines()[-1])
    assert event['event'] == 'refused' and event['reason'] == 'spec', p.stdout

def execute(name, command, expected=0, label=None):
    return pf('box', 'exec', name, '--', '/bin/sh', '-c', command, expected=expected, label=label)

def snapshot():
    rows = []
    for path in sorted([share / '.git', *(share / '.git').rglob('*')]):
        st = path.lstat()
        digest = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else '-'
        rows.append((str(path.relative_to(share)), st.st_mode, st.st_dev, st.st_ino, st.st_nlink,
                     st.st_size, st.st_uid, st.st_gid, st.st_mtime_ns, st.st_ctime_ns, digest))
    return hashlib.sha256(repr(rows).encode()).hexdigest()

def record(run_id):
    data = json.loads((W / 'state/pinfold/runs' / run_id / 'launch.json').read_text())
    say('launch mounts=' + json.dumps(data['mounts'], separators=(',', ':')))
    assert all(m['kind'] in ('readonly', 'share') for m in data['mounts'])
    return data['mounts']

share = W / 'share'; other = W / 'other'; plain = W / 'plain'
for directory in [share, other, plain]: directory.mkdir()
(share / 'file').write_text('initial\n'); (other / 'file').write_text('readonly\n')
try:
    git('init', '-q'); git('add', 'file'); git('commit', '-qm', 'initial')
    (share / '.git/protected').write_text('protected\n')
    main = mount(share)
    for order in [[main, mount(other, '/project/.git', 'readonly')],
                  [mount(other, '/project/.git', 'readonly'), main]]:
        refuse('reordered protected destination', order)
    refuse('nested protected destination', [main, mount(other, '/project/.git/objects', 'readonly')])
    refuse('parent destination', [mount(share, '/work/project'), mount(other, '/work', 'readonly')])
    refuse('protected source alias', [main, mount(share / '.git', '/alias', 'readonly')])
    if (share / '.GIT').exists():
        refuse('case alias source .GIT', [main, mount(share / '.GIT', '/alias', 'readonly')])
    (share / 'nested').mkdir()
    refuse('overlapping writable sources', [main, mount(share / 'nested', '/nested')])
    (share / 'link').symlink_to('nested')
    refuse('nested symlink destination', [main, mount(other, '/project/link/sub', 'readonly')])
    os.link(share / '.git/protected', share / 'protected-alias')
    refuse('protected hardlink', [main]); (share / 'protected-alias').unlink()
    (share / '.git/link').symlink_to('../file')
    refuse('protected symlink', [main]); (share / '.git/link').unlink()
    (plain / '.git').write_text('gitdir: nowhere\n')
    refuse('top-level Git file', [mount(plain)]); (plain / '.git').unlink()
    (plain / '.git').symlink_to(share / '.git')
    refuse('top-level Git symlink', [mount(plain)]); (plain / '.git').unlink()
    (W / 'share-link').symlink_to(share)
    refuse('final share source symlink', [mount(W / 'share-link')])
    refuse('core share contains protected roots', [mount(W)])
    os.link(share / '.git/protected', other / 'cross-alias')
    refuse('cross-share protected hardlink', [main, mount(other, '/other')]); (other / 'cross-alias').unlink()
    (W / 'git-link').symlink_to(share / '.git')
    refuse('readonly source symlink to protected Git', [main, mount(W / 'git-link', '/alias', 'readonly')])
    profile = W / 'config/pinfold/profiles/probe-path'; profile.mkdir(parents=True)
    (profile / 'share').symlink_to(share)
    refuse('selected profile source inside core share', [main], profile='probe-path')
    cache = pathlib.Path(env.get('XDG_CACHE_HOME', pathlib.Path.home() / '.cache'))
    bad_cache = dict(env, XDG_CACHE_HOME=str(cache / 'missing/..'))
    p = pf('box', 'up', input=json.dumps(spec([mount(share, str(cache / 'pinfold'))])), env=bad_cache,
           expected=1, label='normalized init overlap')
    assert json.loads(p.stdout)['reason'] == 'spec' and 'supplied init' in p.stdout
    assert not (W / 'state/pinfold/boxes').exists() and not (W / 'state/pinfold/runs').exists()
    say('layout/alias refusals: no box or run state created')
    (share / 'link').unlink(); (share / 'link').mkdir()
    (W / 'outside').write_text('outside\n'); os.link(W / 'outside', share / 'outside-hardlink')
    (W / 'unmounted').write_text('unmounted-host-fixture\n')
    (share / 'outward').symlink_to(W / 'unmounted')
    (share / 'target').mkdir(); shutil.copy2(BIN, W / 'built-pinfold')
    for i in range(256): os.link(W / 'built-pinfold', share / 'target' / f'linked-{i}')
    say('ordinary hardlinks=256 source=copy of actual target/debug/pinfold sha256=' +
        hashlib.sha256((W / 'built-pinfold').read_bytes()).hexdigest())
    (share / 'bin').mkdir()
    hostile = share / 'bin/chmod'
    hostile.write_text('#!/bin/sh\n/usr/bin/id -u > /project/root-marker\n'); hostile.chmod(0o755)
    s = spec([main, mount(other, '/project/link/sub', 'readonly'), mount(other, '/readonly', 'readonly')], egress={})
    s['image'] = IMAGE
    s['env'].update(GIT_CONFIG_COUNT='1', GIT_CONFIG_KEY_0='core.abbrev', GIT_CONFIG_VALUE_0='11')
    name, run_id = start(s)
    execute(name, 'test ! -e /project/root-marker && /usr/bin/stat -c "socket-mode=%a uid=%u" '
            '/var/host-services/ssh-auth.sock && test "$(/usr/bin/stat -c %a '
            '/var/host-services/ssh-auth.sock)" = 666 && echo hostile-marker-absent')
    execute(name, 'grep -E "^(Uid|Gid|CapBnd|NoNewPrivs):" /proc/self/status; '
            'printf userns-limit=; cat /proc/sys/user/max_user_namespaces')
    execute(name, 'printf "image-PATH=%s\\n" "$PATH"; command -v mount umount unshare truncate; test -r /project/link/sub/file')
    execute(name, 'git -C /project config --get-all safe.directory; cat /project/outside-hardlink')
    execute(name, 'git -C /project rev-parse --show-toplevel && git -C /project config core.abbrev '
            '&& printf ordinary-edit > /project/edit && cat /readonly/file')
    assert (share / 'edit').read_text() == 'ordinary-edit'
    mounts = record(run_id)
    assert any(m['guest'] == '/project/.git' and m['kind'] == 'readonly' for m in mounts)
    before = snapshot(); say('protected snapshot before=' + before)
    attacks = [('append', 'printf x >> /project/.git/protected'),
               ('truncate', 'truncate -s 0 /project/.git/protected'),
               ('unlink', 'rm /project/.git/protected'),
               ('file rename', 'mv /project/.git/protected /project/taken'),
               ('file replace', 'printf x > /tmp/replacement; mv /tmp/replacement /project/.git/protected'),
               ('Git rename', 'mv /project/.git /project/git-moved'),
               ('Git replace', 'mkdir /tmp/replacement-dir; mv -T /tmp/replacement-dir /project/.git'),
               ('Git remove', 'rm -rf /project/.git'),
               ('case alias protected', 'printf x >> /project/.GIT/protected'),
               ('case alias HEAD', 'printf x > /project/.GIT/HEAD')]
    for label, command in attacks:
        p = execute(name, command, expected=None, label=label)
        after = snapshot(); say(f'{label}: protected-snapshot={after} unchanged={after == before}')
        assert p.returncode != 0 and after == before, label
    for label, command in [('remount', 'mount -o remount,rw /project/.git'),
                           ('unmount', 'umount /project/.git'),
                           ('mount-over', 'mount -t tmpfs tmpfs /project/.git'),
                           ('user namespace', 'unshare -Urnm /bin/true'),
                           ('outward symlink', 'cat /project/outward'),
                           ('readonly write', 'printf x > /readonly/file')]:
        assert execute(name, command, expected=None, label=label).returncode != 0, label
    execute(name, 'printf after-denials > /project/edit; git -C /project rev-parse HEAD')
    assert (share / 'edit').read_text() == 'after-denials' and snapshot() == before
    assert (W / 'unmounted').read_text() == 'unmounted-host-fixture\n'
    live = subprocess.Popen([str(BIN), 'box', 'exec', name, '--', '/bin/sh', '-c',
        'echo LIVE_READY; n=0; errors=0; while [ ! -e /project/gate ]; do '
        'git -C /project rev-parse HEAD >/dev/null 2>&1 || errors=$((errors+1)); '
        'printf live > /project/live; n=$((n+1)); done; echo concurrent-reads=$n errors=$errors'], env=env, text=True,
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    say(line_until(live, lambda x: x == 'LIVE_READY'))
    (share / 'file').write_text('host-commit\n'); git('add', 'file'); git('commit', '-qm', 'host-concurrent')
    git('rev-parse', 'HEAD'); after_commit = snapshot(); (share / 'gate').touch()
    output, _ = live.communicate(timeout=90); say(f'concurrent guest rc={live.returncode} ' + output.strip())
    assert live.returncode == 0 and (share / 'live').read_text() == 'live' and snapshot() == after_commit
    execute(name, 'git -C /project log -1 --format=%s; echo guest-Git-status=$?; printf final > /project/edit')
    stop(name)
    alias_before = snapshot()
    name, run_id = start(spec([mount(share, '/run/project'), mount(other, '/var/run/project/.git')]))
    execute(name, 'printf alias > /run/project/.git/protected; echo alias-write-status=$?', label='image-alias visible-source write')
    say('image-alias protected-host-unchanged=' + str(snapshot() == alias_before)); assert snapshot() == alias_before
    stop(name)
    name, run_id = start(spec([mount(plain)]))
    execute(name, 'printf plain-edit > /project/file; test ! -e /project/.git')
    assert (plain / 'file').read_text() == 'plain-edit'
    assert not any(m['guest'].endswith('/.git') for m in record(run_id)); stop(name)
    disk = f'probe-share279-home-{os.getpid()}'; disks.append(disk)
    s = spec([mount(other, '/readonly', 'readonly')], profile='default',
             disks=[dict(name=disk, guest='/home/pi', size='1G')]); s['env']['HOME'] = '/home/pi'
    name, run_id = start(s)
    execute(name, 'cat /readonly/file; cat /opt/pinfold/profile/pi/package.json; '
            'test -f /home/pi/.pi/agent/settings.json')
    assert execute(name, 'printf x > /opt/pinfold/profile/pi/package.json', expected=None,
                   label='profile readonly write').returncode != 0
    assert any(m['guest'] == '/opt/pinfold/profile' and m['kind'] == 'readonly' for m in record(run_id))
    stop(name)
finally:
    for name in list(owners):
        try: stop(name)
        except Exception as error: say('cleanup error: ' + str(error))
    for disk in disks: pf('disk', 'rm', disk)
    assert not owners
    shutil.rmtree(W); say(f'fixture-removed={not W.exists()}')
