import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import time

OUT = Path(__file__).resolve().parent
PINFOLD = '/home/yard/.local/bin/pinfold'
PODMAN = '/usr/local/bin/podman'
BASE_ENV = {k: v for k, v in os.environ.items() if k in {
    'PATH', 'HOME', 'USER', 'LOGNAME', 'XDG_RUNTIME_DIR', 'XDG_DATA_HOME',
    'DBUS_SESSION_BUS_ADDRESS', 'LANG', 'LC_ALL'}}
BASE_ENV['PATH'] = '/usr/local/bin:/usr/bin:/bin'
BASE_ENV['XDG_CACHE_HOME'] = str(OUT / 'cache')
BASE_ENV['XDG_STATE_HOME'] = str(OUT / 'state')
OWNERS = []
REPORT = {'purpose': 'Disposable policy-binding investigation; no production qualification',
          'observations': [], 'cleanup': []}

def digest(data):
    return hashlib.sha256(data).hexdigest()

def run(argv, env=BASE_ENV, check=True):
    return subprocess.run(argv, cwd=OUT, env=env, capture_output=True,
                          timeout=45, check=check)

def policy_file(path, restrictive):
    policy = json.loads(Path('/usr/share/containers/seccomp.json').read_bytes())
    if restrictive:
        for rule in policy['syscalls']:
            if 'getppid' in rule.get('names', []):
                rule['names'].remove('getppid')
        policy['syscalls'] = [r for r in policy['syscalls'] if r.get('names')]
        policy['syscalls'].append({'names': ['getppid'], 'action': 'SCMP_ACT_ERRNO', 'errnoRet': 1})
    temp = path.with_suffix('.tmp')
    temp.write_text(json.dumps(policy))
    temp.replace(path)

def env_for(label, source):
    conf = OUT / (label + '.conf')
    conf.write_text('[containers]\nseccomp_profile = ' + json.dumps(str(source)) + '\n')
    env = dict(BASE_ENV)
    env['CONTAINERS_CONF'] = str(conf)
    return env

def start(label, env):
    name = 'codex-y153-' + label
    spec = {'name': name, 'image': 'pinfold/profile-default:latest',
            'labels': {'operator.investigation': 'y153-policy-binding'},
            'mounts': [{'host': str(OUT / 'fixture'), 'guest': '/probe', 'readonly': True}],
            'env': {}, 'memory': '256M'}
    err = (OUT / (label + '-' + str(len(OWNERS)) + '-owner-stderr.log')).open('xb')
    owner = subprocess.Popen([PINFOLD, 'box', 'up'], cwd=OUT, env=env,
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err)
    item = {'name': name, 'owner': owner, 'env': env, 'err': err, 'pending': b''}
    OWNERS.append(item)
    owner.stdin.write((json.dumps(spec) + '\n').encode())
    owner.stdin.flush()
    return item

def ready(item):
    owner = item['owner']
    selector = selectors.DefaultSelector()
    selector.register(owner.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + 45
    try:
        while time.monotonic() < deadline:
            if not selector.select(max(0, deadline - time.monotonic())):
                break
            data = os.read(owner.stdout.fileno(), 16384)
            if not data:
                break
            item['pending'] += data
            assert len(item['pending']) < 65536
            while b'\n' in item['pending']:
                line, item['pending'] = item['pending'].split(b'\n', 1)
                event = json.loads(line)
                if event['event'] == 'ready':
                    assert event['owner'] == owner.pid
                    item['ready'] = event
                    return
                raise RuntimeError('startup event: ' + json.dumps(event))
        raise RuntimeError('No ready event for ' + item['name'])
    finally:
        selector.close()

def observe(item, label):
    # Own credential-free box only. Never retain whole config or environments.
    raw = run([PODMAN, 'inspect', item['name']], item['env'])
    info = json.loads(raw.stdout)[0]
    path = Path(info['OCIConfigPath'])
    assert path.is_absolute() and path.stat().st_size < 2 * 1024 * 1024
    cfg = json.loads(path.read_bytes())
    policy = cfg.get('linux', {}).get('seccomp')
    data = json.dumps(policy, sort_keys=True, separators=(',', ':')).encode()
    (OUT / (label + '-oci-seccomp.json')).write_bytes(data + b'\n')
    record = {'label': label, 'name': item['name'], 'owner_pid': item['owner'].pid,
              'container_id': info['Id'], 'container_pid': info['State']['Pid'],
              'started_at': info['State']['StartedAt'], 'restart_count': info['RestartCount'],
              'generation': info['Config']['Labels'].get('dev.pinfold.generation'),
              'image_id': info['Image'], 'oci_seccomp_sha256': digest(data),
              'oci_hooks_present': bool(cfg.get('hooks'))}
    crun_dir = Path('/run/user/1000/crun') / info['Id']
    record['crun_state_dir'] = str(crun_dir)
    record['crun_state_files'] = sorted(p.name for p in crun_dir.iterdir()) if crun_dir.exists() else []
    bpf_path = crun_dir / 'seccomp.bpf'
    if bpf_path.exists():
        bpf = bpf_path.read_bytes()
        assert 0 < len(bpf) < 1024 * 1024
        (OUT / (label + '-seccomp.bpf')).write_bytes(bpf)
        record['bpf_sha256'] = digest(bpf)
        record['bpf_bytes'] = len(bpf)
    saved = crun_dir / 'config.json'
    if saved.exists():
        config = json.loads(saved.read_bytes())
        record['crun_no_new_privileges'] = config.get('process', {}).get('noNewPrivileges')
        seccomp = config.get('linux', {}).get('seccomp', {})
        record['crun_seccomp_options'] = {k: seccomp[k] for k in ['flags', 'listenerPath', 'listenerMetadata'] if k in seccomp}
        annotations = config.get('annotations', {})
        record['crun_notification_annotations_present'] = [k for k in ['run.oci.seccomp.plugins', 'run.oci.seccomp.receiver'] if k in annotations]
    result = run([PINFOLD, 'box', 'exec', item['name'], '--', '/probe/syscall-probe'], item['env'], check=False)
    record['exec_exit'] = result.returncode
    assert result.returncode == 0, result.stderr.decode()
    record['syscalls'] = json.loads(result.stdout)
    REPORT['observations'].append(record)
    print(json.dumps(record), flush=True)
    return record

def stop(item):
    owner = item['owner']
    if item.get('stopped'):
        return
    owner.stdin.close()
    owner.wait(timeout=45)
    events = [json.loads(x) for x in (item['pending'] + owner.stdout.read(65536)).splitlines() if x]
    absent = run([PODMAN, 'container', 'exists', item['name']], item['env'], check=False)
    result = {'name': item['name'], 'owner_exit': owner.returncode,
              'down_observed': any(x.get('event') == 'down' for x in events),
              'exists_exit': absent.returncode}
    REPORT['cleanup'].append(result)
    item['err'].close()
    item['stopped'] = True
    assert result['owner_exit'] == 0 and result['down_observed'] and absent.returncode == 1

try:
    REPORT['binary_sha256'] = {p: digest(Path(p).read_bytes()) for p in [PINFOLD, PODMAN, '/usr/bin/crun']}
    (OUT / 'fixture').mkdir()
    c = OUT / 'syscall-probe.c'
    c.write_text('#define _GNU_SOURCE\n#include <errno.h>\n#include <sched.h>\n#include <stdio.h>\n#include <sys/syscall.h>\n#include <unistd.h>\nint main(void) { errno=0; long p=syscall(SYS_getppid); int pe=errno; errno=0; long u=syscall(SYS_unshare,CLONE_NEWUSER); int ue=errno; printf("{\\"getppid\\":%ld,\\"getppid_errno\\":%d,\\"unshare\\":%ld,\\"unshare_errno\\":%d,\\"uid\\":%d}\\n",p,pe,u,ue,getuid()); return 0; }\n')
    run(['/usr/bin/cc', '-O2', '-static', '-o', str(OUT / 'fixture' / 'syscall-probe'), str(c)])
    source = OUT / 'host-policy.json'
    policy_file(source, False)
    env = env_for('host', source)
    a = start('baseline', env)
    ready(a)
    baseline = observe(a, 'baseline')
    assert baseline['syscalls']['getppid'] >= 0 and baseline['syscalls']['getppid_errno'] == 0
    policy_file(source, True)
    b = start('restricted', env)
    ready(b)
    restricted = observe(b, 'restricted')
    old = observe(a, 'baseline-after-host-change')
    assert restricted['syscalls']['getppid'] == -1 and restricted['syscalls']['getppid_errno'] == 1
    assert old['syscalls']['getppid'] >= 0 and old['syscalls']['getppid_errno'] == 0
    # Replace the existing per-user shared cache through a different legitimate launch.
    assert baseline['oci_seccomp_sha256'] == old['oci_seccomp_sha256']
    assert baseline.get('bpf_sha256') == old.get('bpf_sha256')
    stop(a)
    replacement = start('baseline', env)
    ready(replacement)
    replaced = observe(replacement, 'same-name-replacement')
    assert replaced['container_id'] != baseline['container_id']
    assert replaced['syscalls']['getppid'] == -1
    stop(replacement)
    stop(b)
    # Real concurrent launches, sharing only this probe's cache; no wrapper or runtime stub.
    src_a, src_b = OUT / 'policy-a.json', OUT / 'policy-b.json'
    policy_file(src_a, False)
    policy_file(src_b, True)
    env_a, env_b = env_for('a', src_a), env_for('b', src_b)
    x, y = start('concurrent-a', env_a), start('concurrent-b', env_b)
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        list(pool.map(ready, [x, y]))
    obs_a = observe(x, 'concurrent-a')
    obs_b = observe(y, 'concurrent-b')
    REPORT['concurrent_requested_policy_matches'] = {
        'a': obs_a['syscalls']['getppid'] >= 0 and obs_a['syscalls']['getppid_errno'] == 0,
        'b': obs_b['syscalls']['getppid'] == -1 and obs_b['syscalls']['getppid_errno'] == 1}
    REPORT['all_observed_uid_and_namespace_controls_preserved'] = all(
        x['syscalls']['uid'] == os.getuid() and x['syscalls']['unshare'] == -1 and
        x['syscalls']['unshare_errno'] == 1 for x in REPORT['observations'])
    REPORT['status'] = 'completed'
except BaseException as error:
    REPORT['status'] = 'failed'
    REPORT['error'] = str(error)
    raise
finally:
    for item in OWNERS:
        try:
            stop(item)
        except BaseException as error:
            REPORT['cleanup'].append({'name': item['name'], 'error': str(error)})
    REPORT['binary_unchanged'] = all(digest(Path(p).read_bytes()) == h for p, h in REPORT.get('binary_sha256', {}).items())
    (OUT / 'report.json').write_text(json.dumps(REPORT, indent=2) + '\n')
    print(json.dumps({'status': REPORT.get('status'), 'cleanup': REPORT['cleanup'],
                      'binary_unchanged': REPORT['binary_unchanged']}), flush=True)
