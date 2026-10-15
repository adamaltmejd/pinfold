#!/usr/bin/env python3
"""After-suite diagnostic only: frozen-source A build, B metadata, real update ABBA."""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib

BASE = 'ed78cb74fd88a2e1995fb5e499fab7b42f96645b'
root = Path.cwd()
# These are the only reviewed diagnostic additions/replacements. Setup, runtime,
# observer, storage probes, tests and every other tracked file must match BASE.
DECLARED = {'Cargo.toml', '.github/workflows/ci.yml',
            'scripts/sha2-candidate-diagnostic.py', 'scripts/update-abba.py'}

def validate_source():
    changed=set(subprocess.check_output(['git','diff','--name-only','-z',BASE,'--']).decode().split('\0'))-{''}
    untracked=set(subprocess.check_output(['git','ls-files','--others','--exclude-standard','-z']).decode().split('\0'))-{''}
    assert (changed|untracked)<=DECLARED, 'undeclared changed/untracked files: '+repr(sorted((changed|untracked)-DECLARED))
    before=tomllib.loads(subprocess.check_output(['git','show',BASE+':Cargo.toml'],text=True))
    candidate=tomllib.loads((root/'Cargo.toml').read_text())
    expected=json.loads(json.dumps(before))
    expected.setdefault('profile',{}).setdefault('dev',{}).setdefault('package',{})['sha2']={'opt-level':3}
    assert candidate==expected, 'Cargo.toml differs beyond sole sha2 opt-level=3 override'
    assert all((root/name).is_file() for name in DECLARED), 'declared diagnostic source missing'
    return {'baseline_revision':BASE, 'source_revision':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
            'declared_changed_files':sorted(changed|untracked), 'guard':'passed',
            'initial_diagnostic_status':'not-run; inspect later build-provenance.json for attempted outcome'}

if sys.argv[1:]==['--guard-only']:
    print(json.dumps(validate_source(),indent=2))
    raise SystemExit(0)
if sys.argv[1:]:
    raise SystemExit('usage: sha2-candidate-diagnostic.py [--guard-only]')
out = root/'target/issue-92/sha2-diagnostic'
out.mkdir(parents=True, exist_ok=False)
report = {'scope': 'after diagnostic suite timer; no perf; no retries', 'baseline_revision': BASE,
          'source_revision': subprocess.check_output(['git','rev-parse','HEAD'], text=True).strip(),
          'builds': {'A': {'status':'not-run'}, 'B': {'status':'not-run'}}, 'abba':'not-run'}
temporary = Path(tempfile.mkdtemp(prefix='pf92-sha2-', dir=os.environ['RUNNER_TEMP']))

def save():
    (out/'build-provenance.json').write_text(json.dumps(report, indent=2)+'\n')

def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def sources():
    tracked = subprocess.check_output(['git','ls-files','-z','--','Cargo.toml','Cargo.lock',
        'rust-toolchain.toml','.cargo','crates','profile']).decode().split('\0')
    return {name:digest(root/name) for name in tracked if name and (root/name).is_file()}

def build(variant, target_dir, extra):
    row = report['builds'][variant]
    row['status']='attempting'; save()
    started = time.monotonic()
    artifacts = []
    row['sha2_artifacts']=artifacts
    raw = temporary/(variant+'-compiler.jsonl')
    command = ['/usr/bin/time','-f','%e %U %S %M %x','-o',str(out/(variant+'-build.time')),
               'cargo','build','-p','pinfold','--locked','--target',triple,
               '--target-dir',str(target_dir),'--message-format=json',*extra]
    child = None
    try:
        with (out/(variant+'-build.stderr')).open('wb') as stderr, raw.open('wb') as stdout:
            child = subprocess.Popen(command, stdout=stdout, stderr=stderr, start_new_session=True)
            row['exit']=child.wait(timeout=600)
        # The whole build is reaped before parsing; raw compiler messages are never retained.
        with raw.open() as stream:
            for line in stream:
                item = json.loads(line)
                if item.get('reason')!='compiler-artifact' or item.get('target',{}).get('name')!='sha2':
                    continue
                filenames=[Path(name).resolve() for name in item.get('filenames',[])]
                target_root=(target_dir/triple).resolve()
                host_root=(target_dir/'debug').resolve()
                target=bool(filenames) and all(name.is_relative_to(target_root) for name in filenames)
                host=bool(filenames) and all(name.is_relative_to(host_root) for name in filenames)
                assert target != host, 'sha2 compiler-artifact has ambiguous compile_for'
                artifact={key:item.get(key) for key in ('profile','features','fresh')}
                artifact['compile_for']='target' if target else 'host'
                artifacts.append(artifact)
        row['sha2_artifacts']=artifacts
        assert row['exit']==0, f'{variant} build failed'
        assert sum(x['compile_for']=='target' for x in artifacts)==1, 'expected one target sha2 artifact'
        assert sum(x['compile_for']=='host' for x in artifacts)<=1, 'duplicate host sha2 artifact'
        expected='0' if variant=='A' else '3'
        assert all(str(x['profile']['opt_level'])==expected for x in artifacts), 'unexpected sha2 opt_level'
        binary=target_dir/triple/'debug/pinfold'
        row['binary']={'sha256':digest(binary),'size':binary.stat().st_size,
                       'mode':binary.stat().st_mode & 0o7777}
        row['status']='success'
        return binary
    except BaseException as error:
        row['status']='failed'; row['error']=type(error).__name__
        raise
    finally:
        if child is not None and child.poll() is None:
            os.killpg(child.pid,signal.SIGTERM)
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=10)
            # Reaping time alone does not prove every descendant honored TERM.
            try:os.killpg(child.pid,signal.SIGKILL)
            except ProcessLookupError:pass
        if child is not None:row['exit']=child.returncode
        raw.unlink(missing_ok=True)
        row['elapsed_seconds']=time.monotonic()-started
        try:
            wall,user,system,rss,code=(out/(variant+'-build.time')).read_text().splitlines()[-1].split()
            row['command_timing']={'wall_seconds':float(wall),'user_seconds':float(user),
                'system_seconds':float(system),'max_rss_kib':int(rss),'exit':int(code)}
        except (OSError, ValueError, IndexError):
            row['timing_error']='unavailable'
        save()

def interrupted(number, _frame):
    raise KeyboardInterrupt(f'signal {number}')
signal.signal(signal.SIGTERM,interrupted)

try:
    save()
    report['source_guard']=validate_source();save()
    report['source_hashes_start']=sources(); save()
    triple=platform.machine()+'-unknown-linux-musl'
    # Fresh A target is outside the suite target. B metadata is collected without forcing a rebuild.
    a=build('A',temporary/'target',['--config','profile.dev.package.sha2.opt-level=0'])
    b_path=root/'target'/triple/'debug/pinfold'
    report['suite_binary_sha256_before_metadata']=digest(b_path)
    b=build('B',root/'target',[])
    assert digest(b)==report['suite_binary_sha256_before_metadata'], 'B changed after suite'
    def profiles(variant):
        result={}
        for artifact in report['builds'][variant]['sha2_artifacts']:
            profile=artifact['profile'].copy()
            profile.pop('opt_level')
            result[artifact['compile_for']]=profile
        return result
    assert profiles('A')==profiles('B'), 'sha2 non-opt profile fields differ by compile_for'
    fixture=root/'crates/e2e/tests/e2e/release_proxy.py'
    shutil.copyfile(fixture,temporary/'release_proxy.py')
    shutil.copyfile(root/'scripts/update-abba.py',temporary/'update-abba.py')
    assert digest(fixture)==digest(temporary/'release_proxy.py')
    report['fixture_sha256']=digest(fixture)
    report['abba']='attempting';save()
    child=subprocess.Popen([sys.executable,str(temporary/'update-abba.py'),str(a),str(b),str(out/'update-abba')],
        env={**os.environ,'RUNNER_TEMP':str(temporary)},start_new_session=True)
    try:
        result=child.wait(timeout=600)
    finally:
        if child.poll() is None:
            report['abba']='interrupted';save()
            os.killpg(child.pid,signal.SIGTERM)
            try:child.wait(timeout=40)
            except subprocess.TimeoutExpired:
                report['abba_cleanup']='forced-kill; child cleanup unconfirmed';save()
                os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=10)
        report['abba_exit']=child.returncode
        save()
    report['abba']='success' if result==0 else 'failed'
    assert result==0, 'real update ABBA failed'
except BaseException as error:
    report['diagnostic_error']=type(error).__name__
    raise
finally:
    try:
        report['source_hashes_end']=sources()
        report['source_unchanged']=report.get('source_hashes_start')==report['source_hashes_end']
        save()
    finally:
        shutil.rmtree(temporary)
    assert report['source_unchanged'], 'source changed during diagnostic'
