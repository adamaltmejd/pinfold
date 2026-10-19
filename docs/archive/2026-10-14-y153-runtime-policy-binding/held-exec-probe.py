import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess

out = Path(__file__).resolve().parent
env = {k: v for k, v in os.environ.items() if k in ['HOME', 'USER', 'LOGNAME', 'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS', 'LANG']}
env.update(PATH='/usr/local/bin:/usr/bin:/bin', XDG_CACHE_HOME=str(out / 'cache'), XDG_STATE_HOME=str(out / 'state'))
pinfold = '/home/yard/.local/bin/pinfold'
podman = '/usr/local/bin/podman'
name = 'codex-y153-held-exec'
record = {'purpose': 'Bind policy record to one real held exec, not a production gate'}

def sha(data):
    return hashlib.sha256(data).hexdigest()

def run(args):
    return subprocess.run(args, env=env, cwd=out, capture_output=True, timeout=30, check=True)

def line(pipe):
    selector = selectors.DefaultSelector()
    selector.register(pipe, selectors.EVENT_READ)
    try:
        assert selector.select(30), 'No readiness line'
        return json.loads(pipe.readline())
    finally:
        selector.close()

fixture = out / 'fixture'
fixture.mkdir()
source = out / 'held.c'
source.write_text('#define _GNU_SOURCE\n#include <errno.h>\n#include <stdio.h>\n#include <sys/prctl.h>\n#include <sys/syscall.h>\n#include <unistd.h>\nint main(void){errno=0;long p=syscall(SYS_getppid);int e=errno;printf("{\\"guest_pid\\":%d,\\"getppid\\":%ld,\\"errno\\":%d,\\"seccomp\\":%d,\\"no_new_privileges\\":%d}\\n",getpid(),p,e,prctl(PR_GET_SECCOMP,0,0,0,0),prctl(PR_GET_NO_NEW_PRIVS,0,0,0,0));fflush(stdout);if(getchar()!=82)return 6;return 42;}\n')
run(['/usr/bin/cc', '-static', '-O2', '-o', str(fixture / 'held'), str(source)])
record['observer_sha256'] = sha((fixture / 'held').read_bytes())
spec = {'name': name, 'image': 'pinfold/profile-default:latest', 'mounts': [{'host': str(fixture), 'guest': '/probe', 'readonly': True}], 'env': {}, 'memory': '256M'}
owner_err = (out / 'owner-stderr.log').open('xb')
owner = subprocess.Popen([pinfold, 'box', 'up'], cwd=out, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=owner_err)
execution = None
try:
    owner.stdin.write((json.dumps(spec) + '\n').encode())
    owner.stdin.flush()
    ready = line(owner.stdout)
    assert ready['event'] == 'ready' and ready['owner'] == owner.pid, ready
    execution = subprocess.Popen([pinfold, 'box', 'exec', name, '--', '/probe/held'], cwd=out, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    guest = line(execution.stdout)
    info = json.loads(run([podman, 'inspect', name]).stdout)[0]
    sessions = info['ExecIDs']
    assert len(sessions) == 1, sessions
    exec_pid = int((Path(info['OCIConfigPath']).parent / sessions[0] / 'exec_pid').read_text())
    status = {}
    for text in Path('/proc/' + str(exec_pid) + '/status').read_text().splitlines():
        key, _, value = text.partition(':')
        if key in ['NSpid', 'Seccomp', 'Seccomp_filters', 'NoNewPrivs', 'Uid', 'CapEff', 'CapBnd']:
            status[key] = value.strip()
    assert int(status['NSpid'].split()[-1]) == guest['guest_pid']
    live_observer_sha = sha(Path('/proc/' + str(exec_pid) + '/exe').read_bytes())
    assert live_observer_sha == record['observer_sha256']
    state = Path('/run/user/1000/crun') / info['Id']
    bpf = (state / 'seccomp.bpf').read_bytes()
    config = json.loads((state / 'config.json').read_bytes())
    seccomp = config['linux']['seccomp']
    record.update(owner_pid=owner.pid, pinfold_exec_pid=execution.pid,
                  pinfold_exec_sha256=sha(Path('/proc/' + str(execution.pid) + '/exe').read_bytes()),
                  container_id=info['Id'], container_pid=info['State']['Pid'],
                  generation=info['Config']['Labels']['dev.pinfold.generation'],
                  started_at=info['State']['StartedAt'], exec_session=sessions[0],
                  host_exec_pid=exec_pid, guest_observation=guest, host_process_status=status,
                  live_observer_sha256=live_observer_sha, bpf_sha256=sha(bpf),
                  seccomp_options={k: seccomp[k] for k in ['flags', 'listenerPath', 'listenerMetadata'] if k in seccomp},
                  notification_annotations_present=[k for k in ['run.oci.seccomp.plugins', 'run.oci.seccomp.receiver'] if k in config.get('annotations', {})],
                  host_receiver_present='RUN_OCI_SECCOMP_RECEIVER' in env)
    assert guest['seccomp'] == 2 and guest['no_new_privileges'] == 1
    assert status['NoNewPrivs'] == '1' and int(status['CapEff'], 16) == 0 and int(status['CapBnd'], 16) == 0
    execution.stdin.write(b'R\n')
    execution.stdin.flush()
    execution.stdin.close()
    execution.wait(timeout=30)
    record['exec_exit'] = execution.returncode
    assert execution.returncode == 42
    after = json.loads(run([podman, 'inspect', name]).stdout)[0]
    record['lifetime_and_bpf_unchanged'] = all(after['State'][k] == info['State'][k] for k in ['Pid', 'StartedAt']) and after['Id'] == info['Id'] and sha((state / 'seccomp.bpf').read_bytes()) == record['bpf_sha256']
    assert record['lifetime_and_bpf_unchanged']
    record['status'] = 'completed'
except BaseException as error:
    record['status'] = 'failed'
    record['error'] = str(error)
    raise
finally:
    if execution is not None and execution.poll() is None:
        execution.stdin.close()
        execution.wait(timeout=30)
    owner.stdin.close()
    owner.wait(timeout=30)
    events = [json.loads(x) for x in owner.stdout.read(65536).splitlines() if x]
    absent = subprocess.run([podman, 'container', 'exists', name], env=env, capture_output=True, timeout=30)
    record['cleanup'] = {'owner_exit': owner.returncode, 'down_observed': any(x.get('event') == 'down' for x in events), 'exists_exit': absent.returncode}
    owner_err.close()
    (out / 'report.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(record), flush=True)
