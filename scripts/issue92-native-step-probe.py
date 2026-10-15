#!/usr/bin/env python3
"""Post-suite, diagnostic-only Podman steps for the two embedded profiles."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument('--sample', type=int, choices=range(1, 5), required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
output = args.output.resolve()
output.mkdir(parents=True, exist_ok=False)
report_path = output / 'result.json'
order = ('full', 'documents') if args.sample % 2 else ('documents', 'full')
report = {'sample': args.sample, 'order': order,
          'builds': [{'profile': name, 'status': 'not-run'} for name in order],
          'cleanup': [], 'status': 'not-run'}

def save():
    report_path.write_text(json.dumps(report, indent=2) + '\n')

def sha256(data):
    return hashlib.sha256(data).hexdigest()

def interrupted(signum, _frame):
    raise InterruptedError(f'signal {signum}')

signal.signal(signal.SIGTERM, interrupted)

def stop_group(child):
    if child.poll() is not None:
        return
    try:
        os.killpg(child.pid, signal.SIGTERM)
        child.wait(timeout=30)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=10)
    except ProcessLookupError:
        child.wait(timeout=10)

def run_bounded(argv, *, env=None, log=None, timeout=240):
    with (log or Path(os.devnull)).open('wb') as sink:
        child = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL,
                                 stdout=sink, stderr=subprocess.STDOUT,
                                 start_new_session=True)
        try:
            return child.wait(timeout=timeout), False
        except subprocess.TimeoutExpired:
            stop_group(child)
            return child.returncode, True
        except BaseException:
            stop_group(child)
            raise

def storage_snapshot(root):
    result = {}
    try:
        result['graphroot_du_kb'] = int(subprocess.check_output(
            ['du', '-sk', '--', str(root)], text=True, timeout=30).split()[0])
    except Exception as error:
        result['du_error'] = type(error).__name__
    try:
        result['findmnt'] = subprocess.check_output(
            ['findmnt', '--target', str(root), '-n', '-o', 'TARGET,SOURCE,FSTYPE,OPTIONS'],
            text=True, timeout=10).strip()
    except Exception as error:
        result['findmnt_error'] = type(error).__name__
    try:
        result['disk_free_bytes'] = shutil.disk_usage(root).free
    except Exception as error:
        result['disk_error'] = type(error).__name__
    try:
        line = next(line for line in Path('/proc/meminfo').read_text().splitlines()
                    if line.startswith('MemAvailable:'))
        result['mem_available_kb'] = int(line.split()[1])
    except Exception as error:
        result['memory_error'] = type(error).__name__
    return result

save()
try:
    binary = Path('target/x86_64-unknown-linux-musl/debug/pinfold').resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise FileNotFoundError('compiled x64 musl CLI absent after suite')
    if len(os.sched_getaffinity(0)) != 4:
        raise RuntimeError('expected four available CPUs')
    if subprocess.check_output(['podman', '--version'], text=True, timeout=30).strip() != 'podman version 6.1.4':
        raise RuntimeError('pinned Podman version mismatch')
    info = json.loads(subprocess.check_output(['podman', 'info', '--format', 'json'], timeout=30))
    options = info['store']['graphOptions']
    mount_program = options.get('overlay.mount_program')
    if isinstance(mount_program, dict):
        mount_program = mount_program.get('Executable')
    if (info['host']['security']['rootless'] is not True
            or info['store']['graphDriverName'] != 'overlay' or mount_program):
        raise RuntimeError('expected rootless native overlay')
    graph_root = Path(info['store']['graphRoot'])
    report.update({'source': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True, timeout=10).strip(),
                   'binary_sha256': sha256(binary.read_bytes()),
                   'podman': '6.1.4', 'affinity_cpus': 4, 'storage': 'rootless-native-overlay',
                   'graph_root': str(graph_root)})
    fragments = {'full': ('base', 'bun', 'documents', 'full', 'harden'),
                 'documents': ('base', 'bun', 'documents', 'harden')}
    with tempfile.TemporaryDirectory(prefix='pinfold-issue92-step-') as temporary:
        private = Path(temporary)
        for index, name in enumerate(order):
            row = report['builds'][index]
            context = private / f'context-{index}'
            context.mkdir()
            config = private / f'config-{index}'
            config.mkdir()
            env = os.environ.copy()
            env['XDG_CONFIG_HOME'] = str(config)
            exported = f'issue92-{name}-{args.sample}-{secrets.token_hex(4)}'
            status, timed_out = run_bounded(
                [str(binary), 'profile', 'new', exported, '--from', name, '--builtin'],
                env=env, log=output / f'{name}-export.log', timeout=30)
            row['export_exit'], row['export_timed_out'] = status, timed_out
            if status or timed_out:
                row['status'] = 'export-failed'
                save()
                continue
            recipe = (config / 'pinfold' / 'profiles' / exported / 'Containerfile').read_bytes()
            expected = b''.join((Path('profile/image') / f'{part}.Containerfile').read_bytes()
                                for part in fragments[name])
            row['recipe_sha256'] = sha256(recipe)
            row['expected_sha256'] = sha256(expected)
            if recipe != expected:
                row['status'] = 'recipe-mismatch'
                save()
                continue
            (context / 'Containerfile').write_bytes(recipe)
            tag = f'localhost/pinfold-issue92-step-{args.sample}-{index}-{secrets.token_hex(6)}:latest'
            exists = subprocess.run(['podman', 'image', 'exists', tag],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                    timeout=30).returncode
            if exists != 1:
                raise RuntimeError('probe tag is not confirmed absent')
            row['storage_before'] = storage_snapshot(graph_root)
            row['status'] = 'running'
            save()
            started = time.monotonic_ns()
            try:
                status, timed_out = run_bounded(
                    ['podman', 'build', '--layers=false', '--build-arg', 'TARGETARCH=amd64',
                     '--file', str(context / 'Containerfile'), '--tag', tag,
                     '--log-rusage', '--rusage-logfile', str(output / f'{name}-rusage.log'),
                     str(context)], log=output / f'{name}-build.log')
                row.update({'status': 'complete', 'exit': status, 'timed_out': timed_out,
                            'elapsed_ns': time.monotonic_ns() - started})
                if status == 0 and not timed_out:
                    row['storage_after'] = storage_snapshot(graph_root)
                for kind in ('build', 'rusage'):
                    path = output / f'{name}-{kind}.log'
                    row[f'{kind}_log_bytes'] = path.stat().st_size if path.exists() else 0
            finally:
                cleanup = {'profile': name, 'tag': tag}
                try:
                    present = subprocess.run(['podman', 'image', 'exists', tag],
                                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                             timeout=30).returncode
                    cleanup['exists_exit'] = present
                    if present == 0:
                        removed = subprocess.run(['podman', 'image', 'rm', tag],
                                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                                 timeout=60).returncode
                        cleanup['remove_exit'] = removed
                        cleanup['absent_after'] = subprocess.run(
                            ['podman', 'image', 'exists', tag], stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, timeout=30).returncode == 1
                    else:
                        cleanup['absent_after'] = present == 1
                except Exception as error:
                    cleanup['error'] = type(error).__name__
                report['cleanup'].append(cleanup)
                save()
    report['status'] = 'complete'
except BaseException as error:
    report['status'] = 'error'
    report['error'] = type(error).__name__ + ': ' + str(error)
finally:
    save()
if (report['status'] != 'complete' or any(row['status'] != 'complete' or row['exit'] != 0
                                         or row['timed_out'] or row['rusage_log_bytes'] == 0
                                         for row in report['builds'])
        or any(not item.get('absent_after') for item in report['cleanup'])):
    raise SystemExit(1)
