#!/usr/bin/env python3
"""Scratch ABBA actual-binary build-jobs measurement. Not an acceptance gate."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import signal
import shutil
import subprocess
import tempfile
import threading
import time

p = argparse.ArgumentParser()
p.add_argument('--binary-a', required=True)
p.add_argument('--binary-b', required=True)
p.add_argument('--outdir', required=True)
p.add_argument('--source-revision', required=True)
a = p.parse_args()
binaries = {'A': str(Path(a.binary_a).resolve()), 'B': str(Path(a.binary_b).resolve())}
binary = binaries['A']
out = Path(a.outdir).resolve()
out.mkdir(parents=True, exist_ok=True)
if any(out.iterdir()):
    raise SystemExit('Output directory must be empty')
if not re.fullmatch(r'[0-9a-fA-F]{40}', a.source_revision):
    raise SystemExit('--source-revision must be a full Git commit hash')
root = Path(tempfile.mkdtemp(prefix='pf-jobs-', dir='/tmp'))
env = {k: v for k, v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update(XDG_CONFIG_HOME=str(root/'c'), XDG_STATE_HOME=str(root/'s'),
           XDG_CACHE_HOME=str(root/'k'))
for path in ('c', 's', 'k'):
    (root/path).mkdir()
created = []
results = [{'order': i, 'variant': v, 'status': 'not-run',
            'profile': f'pf-jobs-{os.getpid()}-{i}-{v.lower()}'}
           for i,v in enumerate('ABBA', 1)]
cleanup_attempted = set()
cleanup_errors = []
sources = {}
initial_hashes = {}
for variant, path in binaries.items():
    with open(path, 'rb') as stream:
        initial_hashes[variant] = hashlib.file_digest(stream, 'sha256').hexdigest()
assert initial_hashes['A'] != initial_hashes['B'], 'A and B must be distinct actual binaries'
owner = None
owner_name = None
owner_log = None

def interrupted(signum, _frame):
    raise KeyboardInterrupt(f'interrupted by signal {signum}')

signal.signal(signal.SIGTERM, interrupted)


def save(name, value):
    temporary = out/(name+'.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(out/name)


def snapshot():
    cpu = [int(n) for n in Path('/proc/stat').read_text().splitlines()[0].split()[1:]]
    disk = {f[2]: [int(n) for n in f[3:]]
            for line in Path('/proc/diskstats').read_text().splitlines()
            if (f := line.split())}
    return {'monotonic': time.monotonic(), 'cpu_ticks': cpu, 'diskstats': disk,
            'pressure_io': Path('/proc/pressure/io').read_text(),
            'pressure_cpu': Path('/proc/pressure/cpu').read_text()}


def delta(before, after):
    return {'wall_seconds': after['monotonic']-before['monotonic'],
            'cpu_ticks': [y-x for x,y in zip(before['cpu_ticks'],after['cpu_ticks'])],
            'diskstats': {dev: [y-x for x,y in zip(values,after['diskstats'][dev])]
                          for dev,values in before['diskstats'].items()
                          if dev in after['diskstats']}}


def run(argv, stem, timeout=300, check=True):
    began = time.monotonic()
    stdout = b''
    process = None
    problem = None
    try:
        with (out/(stem+'.stderr')).open('wb') as errors:
            process = subprocess.Popen(argv, env=env, cwd=root, stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE, stderr=errors, start_new_session=True)
            try:
                stdout, _ = process.communicate(timeout=timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    stdout, _ = process.communicate(timeout=15)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    stdout, _ = process.communicate(timeout=10)
                raise TimeoutError(f'{stem} exceeded {timeout}s') from None
        if check and process.returncode:
            raise RuntimeError(f'{stem} exited {process.returncode}; inspect its logs')
        return process.returncode, stdout
    except BaseException as error:
        problem = str(error)
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                stdout, _ = process.communicate(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                stdout, _ = process.communicate(timeout=10)
        raise
    finally:
        (out/(stem+'.stdout')).write_bytes(stdout)
        with (out/'commands.jsonl').open('a') as log:
            log.write(json.dumps({'command': stem, 'seconds': time.monotonic()-began,
                'returncode': process.returncode if process else None, 'error': problem})+'\n')


def verify_box_absent(name, stem):
    _, raw = run(['podman', 'ps', '--all', '--format', 'json'], stem+'-boxes', 30)
    for box in json.loads(raw):
        names = box.get('Names') or []
        if isinstance(names, str):
            names = [names]
        assert name not in names, f'Created box remains: {name}'


def cleanup_profile(name, stem):
    if name in cleanup_attempted:
        return
    cleanup_attempted.add(name)
    try:
        verify_box_absent(name+'-box', stem+'-before-remove')
        _, raw = run(['podman', 'image', 'list', '--format', 'json'], stem+'-list', 30)
        prefix = f'pinfold/profile-{name}:'
        tags = sorted({tag for image in json.loads(raw) for tag in image.get('Names') or []
                       if tag.removeprefix('localhost/').startswith(prefix)})
        for index,tag in enumerate(tags):
            run(['podman', 'image', 'rm', tag], stem+f'-remove-{index}', 45)
        _, raw = run(['podman', 'image', 'list', '--format', 'json'], stem+'-verify', 30)
        remaining = [tag for image in json.loads(raw) for tag in image.get('Names') or []
                     if tag.removeprefix('localhost/').startswith(prefix)]
        assert not remaining, f'Created profile tags remain: {remaining}'
        verify_box_absent(name+'-box', stem+'-after-remove')
    except BaseException as error:
        cleanup_errors.append({'profile': name, 'error': str(error)})
        save('cleanup.json', {'errors': cleanup_errors, 'attempted': sorted(cleanup_attempted)})
        raise
    save('cleanup.json', {'errors': cleanup_errors, 'attempted': sorted(cleanup_attempted)})


def teardown():
    global owner, owner_name, owner_log
    if owner is None:
        return
    pending = owner
    name = owner_name
    try:
        if pending.poll() is None:
            # EOF is the ordinary owner-controlled teardown seam.
            pending.stdin.close()
            try:
                pending.wait(timeout=30)
            except subprocess.TimeoutExpired:
                pending.send_signal(signal.SIGTERM)
                try:
                    pending.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    os.killpg(pending.pid, signal.SIGKILL)
                    pending.wait(timeout=10)
        if pending.returncode != 0:
            run([binary, 'box', 'down', name], name+'-failed-down', timeout=45)
        verify_box_absent(name, name+'-teardown')
        if pending.returncode != 0:
            raise RuntimeError(f'box owner {name} exited {pending.returncode}')
    finally:
        owner_log.close()
        owner = None
        owner_name = None


manifest_js = r'''
const fs = require('fs');
const crypto = require('crypto');
const paths = ['/usr/local/bin/bun', '/usr/local/bin/rtk',
 '/opt/pinfold/anydoc', '/opt/pinfold/packages', '/usr/bin/gh'];
if (fs.existsSync('/usr/local/bin/anydoc')) paths.push('/usr/local/bin/anydoc');
const result = [];
function visit(path) {
 const st = fs.lstatSync(path);
 const row = {path, mode: st.mode & 0o7777, uid: st.uid, gid: st.gid};
 if (st.isSymbolicLink()) {row.type='symlink'; row.target=fs.readlinkSync(path);}
 else if (st.isDirectory()) {row.type='directory';}
 else if (st.isFile()) {
  row.type='file'; row.sha256=crypto.createHash('sha256').update(fs.readFileSync(path)).digest('hex');
 } else {throw new Error('Unexpected asset entry type: '+path);}
 result.push(row);
 if (st.isDirectory()) for (const entry of fs.readdirSync(path).sort()) visit(path+'/'+entry);
}
for (const path of paths) visit(path);
console.log(JSON.stringify(result));
'''
save('results.json', results)
save('provenance.json', {'source_revision_argument': a.source_revision,
     'binaries': binaries, 'binary_sha256_start': initial_hashes,
     'labels': {'A': 'unchanged default build jobs', 'B': 'production prototype --jobs=2'},
     'scope': 'Full-profile build/first-launch and asset-equality microbenchmark; not full safety proof'})
try:
    save('host.json', {'uname': list(os.uname()), 'cpu_count': os.cpu_count(),
                       'clock_ticks': os.sysconf('SC_CLK_TCK'),
                       'cpu_models': sorted({line.split(':',1)[1].strip()
                           for line in Path('/proc/cpuinfo').read_text().splitlines()
                           if line.startswith('model name')})})
    run(['podman', 'info', '--format', 'json'], 'runtime', 30)
    baseline = None
    base_digest = None
    for index, variant in enumerate('ABBA', 1):
        binary = binaries[variant]
        current = results[index-1]
        current['status'] = 'attempting'
        save('results.json', results)
        name = current['profile']
        stem = f'{index}-{variant}'
        created.append(name)
        run([binary, 'profile', 'new', name, '--from', 'full', '--builtin'], stem+'-profile', 30)
        source_path = root/'c/pinfold/profiles'/name/'Containerfile'
        source = source_path.read_text()
        if sources:
            assert source == next(iter(sources.values())), 'Containerfile differs across actual binaries or pairs'
        sources[variant] = source
        current['binary_sha256'] = initial_hashes[variant]
        current['containerfile_sha256'] = hashlib.sha256(source.encode()).hexdigest()
        (out/(stem+'.Containerfile')).write_text(source)
        before = snapshot()
        try:
            run([binary, 'build', '--profile', name], stem+'-build', 300)
        finally:
            after = snapshot()
            current['build_seconds'] = delta(before,after)['wall_seconds']
            save(stem+'-build-counters.json', {'before': before, 'after': after, 'delta': delta(before,after)})
            save('results.json', results)
        reference = f'pinfold/profile-{name}:latest'
        _, inspected = run(['podman', 'image', 'inspect', reference], stem+'-inspect', 30)
        info = json.loads(inspected)[0]
        labels = info.get('Labels') or {}
        digest = labels.get('dev.pinfold.base')
        if not digest:
            raise RuntimeError('Built profile lacks resolved base digest')
        if base_digest is None:
            base_digest = digest
        assert digest == base_digest, 'Debian source changed across ABBA builds'
        owner_name = name+'-box'
        owner_log = (out/(stem+'-owner.stderr')).open('wb')
        began = time.monotonic()
        current['first_launch_started'] = began
        save('results.json', results)
        owner = subprocess.Popen([binary, 'box', 'up'], env=env, cwd=root,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=owner_log,
            text=True, start_new_session=True)
        lines = queue.Queue()
        log_path = out/(stem+'-owner.stdout')
        def reader(stream=owner.stdout, target=log_path, channel=lines):
            with target.open('w') as log:
                for line in stream:
                    log.write(line)
                    log.flush()
                    channel.put(line)
            channel.put(None)
        threading.Thread(target=reader, daemon=True).start()
        owner.stdin.write(json.dumps({'name': owner_name, 'image': reference})+'\n')
        owner.stdin.flush()
        line = lines.get(timeout=90)
        if line is None or json.loads(line).get('event') != 'ready':
            raise RuntimeError('Box did not become ready: '+str(line))
        startup_seconds = time.monotonic()-began
        current['first_launch_seconds'] = startup_seconds
        save('results.json', results)
        _, manifest = run([binary, 'box', 'exec', owner_name, '--',
            '/usr/local/bin/bun', '--eval', manifest_js], stem+'-manifest', 90)
        entries = json.loads(manifest)
        save(stem+'-manifest.json', entries)
        if baseline is None:
            baseline = entries
        assert entries == baseline, f'{stem} asset manifest differs from first A'
        run([binary, 'box', 'exec', owner_name, '--', '/usr/local/bin/rtk', '--version'],
            stem+'-rtk-version', 30)
        teardown()
        current.update(status='success', base_digest=digest,
                       asset_manifest_identical=True, asset_entries=len(entries))
        save('results.json', results)
        try:
            cleanup_profile(name, stem+'-cleanup')
        except BaseException as error:
            current.update(status='cleanup-failed', error=str(error))
            save('results.json', results)
            raise
    assert len(sources) == 2 and all(r['status'] == 'success' for r in results)
    print(json.dumps(results, indent=2))
except BaseException as error:
    for record in results:
        if record['status'] == 'attempting':
            record.update(status='failed', error=str(error))
            if 'first_launch_started' in record and 'first_launch_seconds' not in record:
                record['first_launch_failed_after_seconds'] = time.monotonic()-record['first_launch_started']
    save('results.json', results)
    raise
finally:
    try:
        try:
            teardown()
        except BaseException as error:
            cleanup_errors.append({'box': owner_name, 'error': str(error)})
        for name in created:
            try:
                cleanup_profile(name, name+'-final-cleanup')
            except BaseException:
                pass
    finally:
        final_hashes = {}
        binary_end_errors = {}
        for variant, path in binaries.items():
            try:
                with open(path, 'rb') as stream:
                    final_hashes[variant] = hashlib.file_digest(stream, 'sha256').hexdigest()
            except OSError as error:
                final_hashes[variant] = None
                binary_end_errors[variant] = str(error)
        provenance = {'source_revision_argument': a.source_revision, 'binaries': binaries,
            'binary_sha256_start': initial_hashes, 'binary_sha256_end': final_hashes,
            'binary_end_errors': binary_end_errors,
            'binary_unchanged': final_hashes == initial_hashes,
            'labels': {'A': 'unchanged default build jobs', 'B': 'production prototype --jobs=2'},
            'scope': 'Full-profile build/first-launch and asset-equality microbenchmark; not full safety proof'}
        save('provenance.json', provenance)
        save('cleanup.json', {'errors': cleanup_errors, 'attempted': sorted(cleanup_attempted)})
        if not cleanup_errors:
            shutil.rmtree(root)
        else:
            save('retained-workspace.json', {'path': str(root),
                 'reason': 'Cleanup failure; preserve owner state for recovery'})
        if cleanup_errors:
            raise RuntimeError('Bounded cleanup failed; inspect cleanup.json')
        assert provenance['binary_unchanged'], 'Candidate binary changed during measurement'
