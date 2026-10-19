import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import uuid

binary = '<checkout>/target/release/pinfold'
root = Path(tempfile.mkdtemp(prefix='pf-review-pipe-', dir='/private/tmp'))
env = {k: v for k, v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update({f'XDG_{kind}_HOME': str(root / kind.lower()) for kind in ['STATE', 'CACHE', 'CONFIG']})
state = root / 'state' / 'pinfold'
state.mkdir(parents=True)
(state / 'maintenance').touch()
name = 'review-closed-stdout-' + uuid.uuid4().hex[:10]
spec = {'name': name, 'image': 'pinfold/profile-default:latest', 'cpus': 2, 'memory': '1G'}
result = {'name': name, 'binary_sha256': hashlib.sha256(Path(binary).read_bytes()).hexdigest()}
def listed():
    data = subprocess.run(['container', 'list', '--all', '--format', 'json'], capture_output=True, check=True, timeout=30)
    return [{'id': r['id'], 'state': r['status']['state']} for r in json.loads(data.stdout) if r['id'] == name]
try:
    error_file = (root / 'owner.stderr').open('w+b')
    owner = subprocess.Popen([binary, 'box', 'up'], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=error_file)
    owner.stdout.close()
    owner.stdin.write(json.dumps(spec).encode() + b'\n')
    owner.stdin.flush()
    result['owner_exit'] = owner.wait(timeout=45)
    error_file.seek(0)
    result['owner_stderr'] = error_file.read().decode(errors='replace')
    owner.stdin.close()
    result['native_after_owner_exit'] = listed()
finally:
    cleanup = subprocess.run([binary, 'box', 'down', name], env=env, capture_output=True, timeout=35)
    result['cleanup_exit'] = cleanup.returncode
    result['cleanup_stderr'] = cleanup.stderr.decode(errors='replace')
    result['native_after_cleanup'] = listed()
    Path('/private/tmp/pinfold-review-closed-stdout.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))
    if not result['native_after_cleanup']:
        shutil.rmtree(root)
