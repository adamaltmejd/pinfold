import hashlib
import json
import os
from pathlib import Path
import selectors
import shlex
import shutil
import subprocess
import tempfile
import time

binary = '<checkout>/target/release/pinfold'
root = Path(tempfile.mkdtemp(prefix='pf-review-hooks-', dir='/private/tmp'))
project = root / 'project'
project.mkdir()
hooks = project / 'hooks'
hooks.mkdir()
alias = root / 'external-hooks'
alias.symlink_to(hooks, target_is_directory=True)
marker = root / 'host-hook-ran'
hook = hooks / 'pre-commit'
hook.write_text('#!/bin/sh\nexit 0\n')
hook.chmod(0o755)
env = {k: v for k, v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update({f'XDG_{kind}_HOME': str(root / kind.lower()) for kind in ['STATE', 'CACHE', 'CONFIG']})
env.update({'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_NOSYSTEM': '1', 'PINFOLD_ALLOW': '',
            'PINFOLD_CPUS': '2', 'PINFOLD_MEMORY': '1G'})
state = root / 'state' / 'pinfold'
state.mkdir(parents=True)
(state / 'maintenance').touch()
result = {'binary_sha256': hashlib.sha256(Path(binary).read_bytes()).hexdigest(), 'scenarios': []}

def run(args, data=None, check=True):
    return subprocess.run(args, env=env, cwd=project, input=data, capture_output=True, timeout=45, check=check)

def listing():
    return [json.loads(line) for line in run([binary, 'box', 'list', '--label', 'dev.pinfold.owner']).stdout.splitlines()]

try:
    run(['git', 'init', '-q'])
    run(['git', 'config', 'user.name', 'Pinfold review fixture'])
    run(['git', 'config', 'user.email', 'fixture@example.invalid'])
    for mode, path in [('external_symlink', alias), ('direct_inside_control', hooks)]:
        run(['git', 'config', 'core.hooksPath', str(path)])
        marker.unlink(missing_ok=True)
        hook.write_text('#!/bin/sh\nexit 0\n')
        row = {'mode': mode, 'configured_path': str(path), 'resolved_path': str(path.resolve())}
        result['scenarios'].append(row)
        error_path = root / f'{mode}.stderr'
        with error_path.open('wb') as err:
            owner = subprocess.Popen([binary, 'pi', '--mode', 'rpc'], env=env, cwd=project,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err)
        box_name = None
        try:
            owner.stdin.write(b'{"type":"get_state","id":"review"}\n')
            owner.stdin.flush()
            deadline = time.monotonic() + 150
            with selectors.DefaultSelector() as selector:
                selector.register(owner.stdout, selectors.EVENT_READ)
                while True:
                    if not selector.select(max(0, deadline-time.monotonic())):
                        raise RuntimeError('Pi RPC readiness deadline')
                    raw = owner.stdout.readline()
                    if not raw:
                        raise RuntimeError('Pi ended before RPC reply: '+error_path.read_text(errors='replace'))
                    try:
                        reply = json.loads(raw)
                    except json.JSONDecodeError:
                        continue
                    if reply.get('id') == 'review':
                        row['rpc_ready'] = True
                        break
            owned = [x for x in listing() if x['owner'] == owner.pid]
            if len(owned) != 1:
                raise RuntimeError(f'expected one owned box, got {len(owned)}')
            box_name = owned[0]['name']
            row['box'] = box_name
            payload = '#!/bin/sh\nprintf review-hook-executed > '+shlex.quote(str(marker))+'\n'
            write = run([binary, 'box', 'exec', box_name, '--', 'sh', '-c', 'cat > "$1"', 'sh', str(hook)],
                        payload.encode(), check=False)
            row['guest_hook_write_exit'] = write.returncode
            row['guest_hook_write_stderr'] = write.stderr.decode(errors='replace')
            work = run([binary, 'box', 'exec', box_name, '--', 'sh', '-c', 'printf allowed > "$1"', 'sh', str(project/'ordinary')])
            row['ordinary_write_exit'] = work.returncode
            row['host_ordinary_bytes'] = (project/'ordinary').read_text()
            commit = run(['git', '-c', 'commit.gpgsign=false', '-c', 'core.fsmonitor=false', 'commit', '--allow-empty', '-m', 'Review fixture'], check=False)
            row['host_commit_exit'] = commit.returncode
            row['host_marker_exists'] = marker.exists()
            row['host_marker'] = marker.read_text() if marker.exists() else None
        finally:
            owner.stdin.close()
            try:
                row['owner_exit'] = owner.wait(timeout=30)
            except subprocess.TimeoutExpired:
                owner.terminate()
                row['owner_exit'] = owner.wait(timeout=30)
            if box_name:
                row['cleanup_exit'] = run([binary, 'box', 'down', box_name], check=False).returncode
                row['remaining_box'] = any(x['name'] == box_name for x in listing())
            row['owner_stderr'] = error_path.read_text(errors='replace')
        print(json.dumps(row), flush=True)
finally:
    Path('/private/tmp/pinfold-review-hooks.json').write_text(json.dumps(result, indent=2))
    if all(not row.get('remaining_box', True) for row in result['scenarios']):
        shutil.rmtree(root)
