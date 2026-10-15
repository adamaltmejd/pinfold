"""Same-byte CLI placement experiment, after the complete diagnostic suite."""
import hashlib
import json
import os
from pathlib import Path
import resource
import selectors
import shutil
import subprocess
import tempfile
import time

source = Path('target/x86_64-unknown-linux-musl/debug/pinfold').resolve()
output = Path('target/issue-92-profile/staging-abba.json')
records = []

def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def inventory(directory):
    counts = {'files': 0, 'directories': 0, 'symlinks': 0}
    for root, dirs, files in os.walk(directory, followlinks=False):
        for name in dirs + files:
            path = Path(root) / name
            key = 'symlinks' if path.is_symlink() else 'directories' if path.is_dir() else 'files'
            counts[key] += 1
    return counts

report = {'design': 'A original Cargo directory; B identical CLI alone; ABBA ABBA after suite',
          'results': records}
with tempfile.TemporaryDirectory(prefix='pf92-') as temporary:
    root = Path(temporary)
    directory = root / 'binary'
    directory.mkdir()
    staged = directory / 'pinfold'
    start = time.monotonic()
    shutil.copy2(source, staged)
    report['copy_seconds'] = time.monotonic() - start
    a, b = source.stat(), staged.stat()
    report['provenance'] = {
        'sha256_equal': digest(source) == digest(staged),
        'sha256': digest(source), 'sizes': [a.st_size, b.st_size],
        'modes': [a.st_mode, b.st_mode],
        'distinct_inode': (a.st_dev, a.st_ino) != (b.st_dev, b.st_ino),
        'original_export_entries': inventory(source.parent),
        'staged_export_entries': inventory(staged.parent),
    }
    assert report['provenance']['sha256_equal'] and report['provenance']['distinct_inode']
    for index, choice in enumerate('ABBAABBA', 1):
        binary = source if choice == 'A' else staged
        fixture = root / str(index)
        fixture.mkdir()
        env = os.environ.copy()
        for kind in ('STATE', 'CONFIG', 'CACHE'):
            path = fixture / kind.lower()
            path.mkdir()
            env[f'XDG_{kind}_HOME'] = str(path)
        name = f'pf92-stage-{os.getpid()}-{index}'
        row = {'index': index, 'variant': choice}
        records.append(row)
        child = None
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        started = time.monotonic()
        try:
            with (fixture / 'stderr').open('wb') as errors:
                child = subprocess.Popen([str(binary), 'box', 'up'], env=env,
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors)
                child.stdin.write((json.dumps({'name': name, 'image': 'pinfold/profile-default:latest'})+'\n').encode())
                child.stdin.flush()
                with selectors.DefaultSelector() as selector:
                    selector.register(child.stdout, selectors.EVENT_READ)
                    deadline = time.monotonic() + 90
                    buffered = b''
                    while b'\n' not in buffered:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0 or not selector.select(remaining):
                            raise TimeoutError('box readiness observation exceeded 90 seconds')
                        piece = os.read(child.stdout.fileno(), 65536)
                        if not piece or len(buffered) + len(piece) > 65536:
                            raise ValueError('missing or oversized readiness line')
                        buffered += piece
                    line, buffered = buffered.split(b'\n', 1)
                    ready = json.loads(line)
                row['ready_seconds'] = time.monotonic() - started
                row['ready_event'] = ready.get('event')
                assert ready.get('event') == 'ready', ready.get('reason')
                executed = subprocess.run([str(binary), 'box', 'exec', name, '--', '/bin/true'],
                    env=env, capture_output=True, timeout=60)
                row['exec_exit'] = executed.returncode
                assert executed.returncode == 0
                child.stdin.close()
                child.stdin = None
                stdout, _ = child.communicate(timeout=60)
                row['owner_exit'] = child.returncode
                events = [json.loads(line).get('event') for line in (buffered + stdout).splitlines()]
                row['down_events'] = events
                assert child.returncode == 0 and events == ['down']
        except Exception as error:
            row['error'] = type(error).__name__ + ': ' + str(error)
            raise
        finally:
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
            after = resource.getrusage(resource.RUSAGE_CHILDREN)
            row['total_seconds'] = time.monotonic() - started
            row['child_user_seconds'] = after.ru_utime - before.ru_utime
            row['child_system_seconds'] = after.ru_stime - before.ru_stime
            try:
                down = subprocess.run([str(binary), 'box', 'down', name], env=env,
                    capture_output=True, timeout=30)
                row['cleanup_down_exit'] = down.returncode
                exists = subprocess.run(['podman', 'container', 'exists', name],
                    capture_output=True, timeout=30)
                row['cleanup_exists_exit'] = exists.returncode
                if exists.returncode == 0:
                    removed = subprocess.run(['podman', 'rm', '--force', name],
                        capture_output=True, timeout=30)
                    row['orphan_removal_exit'] = removed.returncode
                assert down.returncode == 0 and exists.returncode == 1, 'unexpected leftover box'
            finally:
                output.write_text(json.dumps(report, indent=2)+'\n')
    report['source_hash_unchanged'] = digest(source) == report['provenance']['sha256']
    report['staged_hash_unchanged'] = digest(staged) == report['provenance']['sha256']
    assert report['source_hash_unchanged'] and report['staged_hash_unchanged']
output.write_text(json.dumps(report, indent=2)+'\n')
