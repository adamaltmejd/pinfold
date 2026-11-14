#!/usr/bin/env python3
"""Manual PR 2 collector. Run with PROBE_ROOT=/tmp/pfi.XXXX; no suite tests."""
import datetime, fcntl, json, os, pathlib, pty, re, select, shutil, struct
import subprocess, sys, termios, time

ROOT = pathlib.Path(__file__).resolve().parents[3]
LINUX = sys.platform.startswith('linux')
P = pathlib.Path(os.environ.get('PINFOLD_BIN', ROOT / 'target' /
    ('x86_64-unknown-linux-musl/debug/pinfold' if LINUX else 'debug/pinfold')))
W = pathlib.Path(os.environ['PROBE_ROOT']).resolve()
if not str(W).startswith(('/tmp/pfi.', '/private/tmp/pfi.')) or len(str(W)) > 35:
    raise RuntimeError('use a short, dedicated /tmp/pfi. scratch root')
W.mkdir(exist_ok=True)
ENV = {k: os.environ[k] for k in ('HOME', 'PATH', 'USER', 'LOGNAME', 'TMPDIR', 'XDG_CACHE_HOME') if k in os.environ}
ENV.update(XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'),
           PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G', EDITOR='true')
(W/'s/pinfold').mkdir(parents=True, exist_ok=True)
(W/'s/pinfold/maintenance').touch()
MODE = sys.argv[1]
LOG = W/(MODE+'.log')
OWN = W/'owned.json'
owned = json.loads(OWN.read_text()) if OWN.exists() else {'boxes': [], 'disks': []}

def emit(s):
    s = str(s).replace(str(ROOT), '/home/user/pinfold').replace(str(pathlib.Path.home()), '/home/user')
    print(s, flush=True)
    with LOG.open('a') as f: f.write(s+'\n')

def deadline():
    if LINUX and datetime.datetime.now(datetime.timezone.utc) >= datetime.datetime(2026, 11, 15, 0, 28, tzinfo=datetime.timezone.utc):
        raise RuntimeError('lease safety cutoff reached')

def run(args, timeout=180, cwd=None, check=False, quiet=False):
    deadline()
    emit('$ '+' '.join(map(str, args)))
    start = time.monotonic()
    p = subprocess.run(list(map(str, args)), env=ENV, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    if not quiet: emit(p.stdout+p.stderr)
    emit(f'[exit {p.returncode}; seconds {time.monotonic()-start:.3f}]')
    if check and p.returncode: raise RuntimeError('command failed')
    return p

def build(args, cwd=None, check=True):
    run(['df', '-h' if LINUX else '-g', '/'], check=True)
    if shutil.disk_usage('/').free < 12*1024**3: raise RuntimeError('under 12 GiB free')
    p = run([P, *args], timeout=1200, cwd=cwd)
    for path in re.findall(r'egress log (\S+)', p.stderr):
        counts = {}
        for line in pathlib.Path(path).read_text().splitlines():
            row = json.loads(line)
            key = json.dumps({k:v for k,v in row.items() if k not in ('time', 'timestamp')}, sort_keys=True)
            counts[key] = counts.get(key, 0)+1
        for key, count in sorted(counts.items()): emit(f'audit count={count} {key}')
    if check and p.returncode: raise RuntimeError('build failed')
    return p

def refs(stem):
    if LINUX:
        cache = pathlib.Path(ENV.get('XDG_CACHE_HOME', str(pathlib.Path.home()/'.cache')))/'pinfold/images/catalog.json'
        data = json.loads(cache.read_text()) if cache.exists() else {'references': {}}
        result = {r:i for r,i in data['references'].items() if stem+':' in r}
        for identity in set(result.values()): emit('image '+json.dumps(data['images'][identity], sort_keys=True))
    else:
        p = run(['container', 'image', 'list', '--format', 'json'], quiet=True, check=True)
        result = {i['configuration']['name']:i['id'] for i in json.loads(p.stdout)
                  if stem+':' in i['configuration']['name']}
    emit('refs '+json.dumps(result, sort_keys=True))
    return result

def save_owned(): OWN.write_text(json.dumps(owned))

def up(profile, image=None, home=None, harness=False):
    name = 'probe-pr2-'+W.name[4:]+'-'+profile
    disk = home or name+'-home'
    if disk not in owned['disks']:
        inventory = run([P, 'disk', 'list', '--json'], check=True, quiet=True)
        if any(json.loads(line)['name'] == disk for line in inventory.stdout.splitlines() if line):
            raise RuntimeError('planned disk exists without ownership')
        owned['disks'].append(disk)
    if name not in owned['boxes']: owned['boxes'].append(name)
    save_owned()
    spec = {'name': name, 'profile': profile, 'cpus': 2, 'memory': '1G',
            'env': {'HOME': '/home/user'}, 'disks': [{'name': disk, 'guest': '/home/user'}],
            'egress': {'allow': [], 'routes': {}}}
    if image: spec['image'] = image
    if harness: spec['harnesses'] = ['pi', 'claude', 'codex']
    emit('spec '+json.dumps(spec))
    err = (W/(name+'.stderr')).open('w')
    owner = subprocess.Popen([str(P), 'box', 'up'], env=ENV, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=err, text=True)
    owner.stdin.write(json.dumps(spec)+'\n'); owner.stdin.flush()
    if not select.select([owner.stdout], [], [], 180)[0]: raise RuntimeError('readiness deadline')
    line = owner.stdout.readline(); emit(line)
    ready = json.loads(line)
    if ready.get('event') != 'ready':
        owner.stdin.close(); owner.wait(timeout=60); err.close()
        emit((W/(name+'.stderr')).read_text()); raise RuntimeError('box did not start')
    return name, owner, err, ready

def x(box, command, **kw): return run([P, 'box', 'exec', box, '--', 'sh', '-c', 'set -eu; '+command], **kw)

def down(box):
    name, owner, err, ready = box
    run([P, 'box', 'down', name], check=True)
    owner.stdin.close(); owner.wait(timeout=90); emit(owner.stdout.read()); err.close()
    emit((W/(name+'.stderr')).read_text())
    if ready['egress_log']:
        for line in pathlib.Path(ready['egress_log']).read_text().splitlines(): emit('startup-audit '+line)

def interactive(box, agent):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 36, 120, 0, 0))
    command = [str(P), 'box', 'exec', box, '--tty', '--', 'sh', '-c', 'cd /tmp; timeout 15 '+agent]
    emit('$ '+' '.join(command)+' [no login choice submitted]')
    proc = subprocess.Popen(command, env={**ENV, 'TERM': 'xterm-256color'}, stdin=slave, stdout=slave,
                            stderr=slave, start_new_session=True)
    os.close(slave); chunks = []; end = time.monotonic()+25
    while time.monotonic() < end:
        if select.select([master], [], [], min(1, max(0, end-time.monotonic())))[0]:
            try: chunks.append(os.read(master, 65536))
            except OSError: break
        if proc.poll() is not None: break
    if proc.poll() is None: proc.terminate()
    try: proc.wait(timeout=5)
    except subprocess.TimeoutExpired: proc.kill(); proc.wait()
    os.close(master)
    text = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07)|\x1b.', '', b''.join(chunks).decode(errors='replace'))
    (W/(agent+'-startup.txt')).write_text(text)
    emit('\n'.join(text.splitlines()[-14:])); emit('observation exit='+str(proc.returncode))

def project():
    path = W/'project'; path.mkdir(exist_ok=True)
    if not (path/'.git').exists():
        run(['git', 'init', '-q', path], check=True)
        run(['git', '-C', path, '-c', 'user.name=user', '-c', 'user.email=user@fixture.invalid',
             'commit', '-q', '--allow-empty', '-m', 'fixture'], check=True)
        run([P, 'config', 'edit'], cwd=path, check=True)
    report = json.loads(run([P, 'config'], cwd=path, check=True).stdout)
    config = pathlib.Path(report['config'])
    config.write_text('mounts = [{ host = ".", kind = "disk" }]\nprofile = "full"\nallow = []\ncpus = 2\nmemory = "1G"\n')
    recipe = config.parent/'Containerfile'
    return path, recipe

if MODE == 'build':
    profile = sys.argv[2]
    for _ in range(int(sys.argv[3]) if len(sys.argv) > 3 else 1): build(['build', '--profile', profile])
    refs('profile-'+profile)
elif MODE == 'start':
    profile = sys.argv[2]
    for agent in ('pi', 'claude', 'codex'):
        box = up(profile)
        try:
            x(box[0], 'command -v pi claude codex; pi --version; claude --version; codex --version; cat /opt/pinfold/agents/install.txt', check=True)
            x(box[0], 'du -sk /usr /opt; find /opt/pinfold/agents -maxdepth 3 -type f -size +30M -printf "%s %p\\n"')
            x(box[0], 'if printf fixture >> "$(command -v '+agent+')"; then echo unexpected-binary-write; exit 1; else echo binary-write-refused; fi', check=True)
            interactive(box[0], agent)
        finally: down(box)
    box = up(profile, harness=True)
    try:
        for agent in ('pi', 'claude', 'codex'):
            run([P, 'box', 'exec', box[0], '--harness', agent, '--', '/opt/pinfold/'+agent+'/'+agent, '--version'], check=True)
    finally: down(box)
elif MODE == 'home':
    profile = sys.argv[2]
    box = up(profile)
    hashes = 'sha256sum /home/user/.pi/agent/settings.json /home/user/.claude/settings.json /home/user/.codex/config.toml'
    try:
        x(box[0], 'jq ".hideThinkingBlock = false" /home/user/.pi/agent/settings.json > /tmp/pi-settings; cat /tmp/pi-settings > /home/user/.pi/agent/settings.json; '
          'jq \'.fixture = "retained"\' /home/user/.claude/settings.json > /tmp/claude-settings; cat /tmp/claude-settings > /home/user/.claude/settings.json; '
          'printf "\\n# fixture retained\\n" >> /home/user/.codex/config.toml', check=True)
        before = x(box[0], hashes, check=True).stdout
    finally: down(box)
    build(['build', '--profile', profile])
    box = up(profile)
    try: emit('edited-home-preserved='+str(before == x(box[0], hashes, check=True).stdout))
    finally: down(box)
elif MODE == 'project':
    path, recipe = project()
    recipe.write_text('FROM localhost/pinfold/profile-full:latest\nRUN printf project-ready > /project-fixture\n')
    build(['build'], cwd=path)
    report = json.loads(run([P, 'config'], cwd=path, check=True).stdout)
    box = up('full', report['image'])
    try: x(box[0], 'cat /project-fixture; pi --version; claude --version; codex --version', check=True)
    finally: down(box)
elif MODE == 'fail':
    profile = W/'c/pinfold/profiles/probe-pr2-failure'; profile.mkdir(parents=True, exist_ok=True)
    recipe = profile/'Containerfile'; good = 'FROM localhost/pinfold/profile-full:latest\nRUN printf ready > /fixture\n'
    recipe.write_text(good); build(['build', '--profile', 'probe-pr2-failure'])
    before = refs('profile-probe-pr2-failure')
    recipe.write_text(good+'RUN install /required-agent-missing /usr/local/bin/fixture\n')
    failed = build(['build', '--profile', 'probe-pr2-failure'], check=False)
    emit('profile-failure='+str(failed.returncode != 0)+' refs-preserved='+str(before == refs('profile-probe-pr2-failure')))
    recipe.write_text(good); build(['build', '--profile', 'probe-pr2-failure'])
    path, recipe = project(); report = json.loads(run([P, 'config'], cwd=path, check=True).stdout)
    stem = report['image'].split('/')[-1].split(':')[0]; before = refs(stem)
    if not before: raise RuntimeError('run project mode first for a published control')
    refs('profile-full')
    recipe.write_text('FROM localhost/pinfold/profile-full:latest\nRUN install /required-agent-missing /usr/local/bin/fixture\n')
    failed = build(['build'], cwd=path, check=False)
    emit('project-failure='+str(failed.returncode != 0)+' refs-preserved='+str(before == refs(stem)))
    refs('profile-full')
    recipe.write_text('FROM localhost/pinfold/profile-full:latest\nRUN printf project-ready > /project-fixture\n')
    build(['build'], cwd=path)
elif MODE == 'caller':
    context = W/'caller'; context.mkdir(exist_ok=True)
    recipe = context/'Containerfile'
    recipe.write_text('FROM localhost/pinfold/profile-full:latest\nRUN date +%s%N > /caller-install-time\n')
    for extra in ([], [], ['--no-cache']):
        build(['image', 'build', 'probe-pr2-caller', '--containerfile', recipe, '--context', context, *extra])
        box = up('full', 'localhost/pinfold/image-probe-pr2-caller:latest')
        try: x(box[0], 'cat /caller-install-time', check=True)
        finally: down(box)
elif MODE == 'cleanup':
    for name in owned['boxes']: run([P, 'box', 'down', name], check=True)
    inventory = run([P, 'disk', 'list', '--json'], check=True, quiet=True)
    existing = {json.loads(line)['name'] for line in inventory.stdout.splitlines() if line}
    for name in owned['disks']:
        if name in existing: run([P, 'disk', 'rm', name], check=True)
    run([P, 'disk', 'list', '--json'], check=True)
else: raise RuntimeError('unknown mode')
