#!/usr/bin/env python3
"""Temporary fixed Tokio admission ACCA diagnostic; never shipping source."""
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time

BASE = '9f5b59b31b88d342d489c0b597040d8b66e30caf'
DRIVER_PATHS = {'.github/workflows/ci.yml', 'scripts/release-scheduling-abba.py', 'scripts/release-scheduling-variants.json'}
PAYLOAD_SHA256 = 'f26302c1de48d3138ff3363dfa26983e45beb3974b59e6cc4c3bb572f28921b2'
WORKFLOW_SHA256 = 'e8d21c5ace901d403da0ceef03f46658af5ed397bbffc96db25f8a0781636e04'
ROOT = Path(os.environ['PINFOLD_SCHEDULING_EVIDENCE'])
ORDER = ('A', 'C', 'C', 'A')

def sha(data):
    return hashlib.sha256(data).hexdigest()

def git(*args):
    return subprocess.check_output(['git', *args])

def names(data):
    return set(data.decode().split('\0')) - {''}

def record(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def interrupt(signum, _frame):
    raise KeyboardInterrupt(f'signal {signum}')

def stop(proc):
    if proc is None:
        return
    # Only the session created for this invocation. The leader may exit first.
    def group_alive():
        try:
            os.killpg(proc.pid, 0)
            return True
        except ProcessLookupError:
            return False
    def send(signum):
        try:
            os.killpg(proc.pid, signum)
        except ProcessLookupError:
            pass
    if group_alive():
        send(signal.SIGTERM)
        deadline = time.monotonic() + 20
        while group_alive() and time.monotonic() < deadline:
            proc.poll()
            time.sleep(0.05)
        if group_alive():
            send(signal.SIGKILL)
            deadline = time.monotonic() + 5
            while group_alive() and time.monotonic() < deadline:
                proc.poll()
                time.sleep(0.05)
    if proc.poll() is None:
        proc.wait(timeout=20)
    assert not group_alive(), 'owned process group remains after termination'

def run_command(argv, directory, kind, cwd=None, env=None):
    proc = None
    began = time.monotonic_ns()
    result = {'status': 'not-run'}
    try:
        with (directory / (kind + '.log')).open('wb') as log:
            proc = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                    stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            code = proc.wait()
        result = {'status': 'complete', 'exit': code, 'elapsed_ns': time.monotonic_ns() - began}
        return code
    finally:
        try:
            stop(proc)
        finally:
            if result['status'] != 'complete':
                result = {'status': 'interrupted', 'elapsed_ns': time.monotonic_ns() - began}
            record(directory / (kind + '-result.json'), result)

def observed_suite(directory):
    observer = None
    suite_env = os.environ.copy()
    suite_env['RUST_TEST_NOCAPTURE'] = '1'
    with (directory / 'host.jsonl').open('wb') as output, (directory / 'observer.stderr').open('wb') as error:
        try:
            observer = subprocess.Popen([sys.executable, 'scripts/diagnose-suite-host.py'],
                                        stdin=subprocess.DEVNULL, stdout=output, stderr=error, start_new_session=True)
            return run_command(['/usr/bin/time', '-f', 'elapsed=%e user=%U system=%S max_rss_kib=%M exit=%x',
                                '-o', str(directory / 'command-time.txt'), sys.executable, 'scripts/e2e-budget.py'],
                               directory, 'suite', env=suite_env)
        finally:
            try:
                stop(observer)
            finally:
                record(directory / 'observer-result.json', {'status': 'complete' if observer is not None else 'not-run',
                                                           'exit': observer.returncode if observer is not None else None})

def load_variants():
    payload = Path('scripts/release-scheduling-variants.json').read_bytes()
    assert sha(payload) == PAYLOAD_SHA256, 'variant payload changed'
    assert sha(Path('.github/workflows/ci.yml').read_bytes()) == WORKFLOW_SHA256, 'workflow changed'
    variants = json.loads(payload)
    assert variants['baseline'] == BASE
    paths = set(variants['files'])
    assert paths == set(['Cargo.lock', 'crates/e2e/Cargo.toml', 'crates/e2e/tests/e2e/box_.rs', 'crates/e2e/tests/e2e/cli.rs', 'crates/e2e/tests/e2e/main.rs', 'crates/e2e/tests/e2e/runtime.rs', 'crates/e2e/tests/e2e/update.rs'])
    assert not (paths & DRIVER_PATHS)
    result = {'A': {}, 'C': {}}
    for name, values in variants['files'].items():
        path = Path(name)
        assert not path.is_absolute() and '..' not in path.parts
        for variant in ('A', 'C'):
            data = values[variant]['text']
            encoded = data.encode() if data is not None else None
            assert (sha(encoded) if encoded is not None else None) == values[variant]['sha256']
            result[variant][name] = encoded
        proc = subprocess.run(['git', 'cat-file', '-e', f'{BASE}:{name}'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        baseline = git('show', f'{BASE}:{name}') if proc.returncode == 0 else None
        assert result['A'][name] == baseline, 'A differs from baseline: ' + name
    return result

def verify_source(expected):
    for name, data in expected.items():
        actual = Path(name).read_bytes() if Path(name).exists() else None
        assert actual == data, 'unexpected variant source: ' + name
    dirty = names(git('diff', '--name-only', '-z', 'HEAD', '--'))
    untracked = names(git('ls-files', '--others', '--exclude-standard', '-z'))
    assert dirty | untracked <= set(expected), 'undeclared working source changes'
    assert not git('diff', '--cached', '--name-only', '-z'), 'staged source edits'

def apply_variant(expected):
    for name, data in expected.items():
        path = Path(name)
        if data is None:
            path.unlink(missing_ok=True)
        elif not path.exists() or path.read_bytes() != data:
            path.write_bytes(data)
    verify_source(expected)

def inventory(directory, label):
    raw = subprocess.check_output(['podman', 'ps', '--all', '--filter', 'label=dev.pinfold.owner', '--format', 'json'], timeout=90)
    values = json.loads(raw)
    record(directory / (label + '-inventory.json'), values)
    assert values == [], 'owned runtime boxes remain; stop before contaminating the next cell'

def main():
    ROOT.mkdir(parents=True, exist_ok=True)
    assert platform.machine() == 'x86_64' and len(os.sched_getaffinity(0)) == 4
    assert not any(os.environ.get(k) for k in ('RUST_TEST_THREADS', 'CARGO_BUILD_JOBS', 'CARGO_TARGET_DIR'))
    changed = names(git('diff', '--name-only', '-z', BASE, 'HEAD', '--'))
    assert changed == DRIVER_PATHS, 'unexpected diagnostic driver changes'
    variants = load_variants()
    verify_source(variants['A'])
    record(ROOT / 'source.json', {'base': BASE, 'head': git('rev-parse', 'HEAD').decode().strip(),
                                 'order': ORDER, 'affinity': sorted(os.sched_getaffinity(0)),
                                 'comparison': 'Both C cells versus final A; initial A retained but excluded from gain claims',
                                 'payload_sha256': PAYLOAD_SHA256, 'workflow_sha256': WORKFLOW_SHA256})
    if '--verify-only' in sys.argv:
        return 0
    for number, variant in enumerate(ORDER, 1):
        directory = ROOT / f'{number}-{variant}'
        directory.mkdir()
        for kind in ('suite', 'observer', 'g35'):
            record(directory / (kind + '-result.json'), {'status': 'not-run'})
    failed = False
    binary = Path.cwd() / 'target/x86_64-unknown-linux-musl/debug/pinfold'
    binary_sha = None
    inventory(ROOT, 'initial')
    try:
        for number, variant in enumerate(ORDER, 1):
            directory = ROOT / f'{number}-{variant}'
            apply_variant(variants[variant])
            record(directory / 'source.json', {'variant': variant, 'sha256': {p: sha(b) if b is not None else None for p, b in variants[variant].items()}})
            suite = observed_suite(directory)
            observer = json.loads((directory / 'observer-result.json').read_text())
            failed |= suite != 0 or observer['exit'] != 0
            digest = sha(binary.read_bytes()) if binary.is_file() else None
            record(directory / 'binary.json', {'sha256': digest})
            assert digest is not None, 'suite binary absent'
            if binary_sha is None:
                binary_sha = digest
            assert digest == binary_sha, 'production binary bytes changed between variants'
            inventory(directory, 'post-suite')
            g35 = run_command(['bash', '-c', 'bun install --frozen-lockfile && bun run check && bun durable_recovery_preserves_boxed_execution.ts --pinfold "$1" --image pinfold/profile-default:latest',
                               'pinfold-scheduling-g35', str(binary)], directory, 'g35', cwd='examples/pi-durable')
            failed |= g35 != 0
            inventory(directory, 'post-g35')
            verify_source(variants[variant])
            print(json.dumps({'cell': f'{number}-{variant}', 'suite_exit': suite, 'observer_exit': observer['exit'], 'g35_exit': g35}), flush=True)
        return int(failed)
    finally:
        apply_variant(variants['A'])
        record(ROOT / 'restoration.json', {'restored_A': True})

if __name__ == '__main__':
    signal.signal(signal.SIGINT, interrupt)
    signal.signal(signal.SIGTERM, interrupt)
    try:
        code = main()
    except BaseException as exc:
        ROOT.mkdir(parents=True, exist_ok=True)
        record(ROOT / 'fatal.json', {'type': type(exc).__name__, 'message': str(exc)})
        raise
    sys.exit(code)
