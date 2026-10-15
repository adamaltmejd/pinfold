#!/usr/bin/env python3
"""Select storage for an explicitly requested fresh disposable CI probe."""
import json
import os
from pathlib import Path
import shutil
import tempfile
import tomllib

mode = os.environ.get('ISSUE92_STORAGE')
if mode not in ('native', 'fuse'):
    raise SystemExit('ISSUE92_STORAGE must be native or fuse')
if os.environ.get('GITHUB_ACTIONS') != 'true' or os.getuid() == 0:
    raise SystemExit('This probe requires a nonroot GitHub Actions job')
if 'CONTAINERS_STORAGE_CONF' in os.environ:
    raise SystemExit('Refusing to replace an existing CONTAINERS_STORAGE_CONF selection')
for name in ('RUNNER_TEMP', 'GITHUB_ENV', 'HOME'):
    if not os.environ.get(name) or any(c in os.environ[name] for c in ('\n', '\r', '\0')):
        raise SystemExit(f'{name} must be a nonempty single-line path')
home = Path(os.environ['HOME'])
if not home.is_absolute():
    raise SystemExit('HOME must be absolute')
data = Path(os.environ.get('XDG_DATA_HOME') or home/'.local/share')
runtime = Path(f'/run/user/{os.getuid()}')
if os.environ.get('XDG_RUNTIME_DIR') and Path(os.environ['XDG_RUNTIME_DIR']) != runtime:
    raise SystemExit('Existing XDG_RUNTIME_DIR differs from the setup script runroot')
if not data.is_absolute() or not runtime.is_absolute():
    raise SystemExit('Default rootless storage parents must be absolute')
graphroot = data/'containers/storage'
runroot = runtime/'containers'
for name, path in (('graphroot', graphroot), ('runroot', runroot)):
    if path.is_symlink():
        raise SystemExit(f'Refusing symlinked {name}: {path}')
    if path.exists() and (not path.is_dir() or any(path.iterdir())):
        raise SystemExit(f'Refusing pre-existing nonempty {name}: {path}')
helper = Path('/usr/bin/fuse-overlayfs')
if not helper.is_file() or not os.access(helper, os.X_OK):
    raise SystemExit('Both cells must install /usr/bin/fuse-overlayfs first')
if mode == 'fuse' and not os.access('/dev/fuse', os.R_OK | os.W_OK):
    raise SystemExit('FUSE probe requires an accessible /dev/fuse; do not change device permissions')
vendor = Path('/usr/share/containers/storage.conf')
source = vendor.read_text()
slots = {'# driver = "overlay"': 'driver = "overlay"',
         '#mount_program = "/usr/bin/fuse-overlayfs"':
         'mount_program = ' + json.dumps(str(helper) if mode == 'fuse' else '')}
lines = source.splitlines(keepends=True)
for old, replacement in slots.items():
    matches = [i for i,line in enumerate(lines) if line.rstrip('\r\n') == old]
    if len(matches) != 1:
        raise SystemExit(f'Pinned vendor config must contain exactly one original slot: {old}')
    i = matches[0]
    lines[i] = replacement + ('\n' if lines[i].endswith('\n') else '')
configured = ''.join(lines)
original = tomllib.loads(source)
actual = tomllib.loads(configured)
expected = tomllib.loads(source)
expected['storage']['driver'] = 'overlay'
expected['storage']['options']['overlay']['mount_program'] = str(helper) if mode == 'fuse' else ''
if actual != expected:
    raise SystemExit('Configuration changed beyond driver and mount_program')
if actual['storage']['options']['overlay'].get('mountopt') != 'nodev':
    raise SystemExit('Pinned overlay mountopt must remain nodev')
for key in ('graphroot', 'runroot', 'rootless_storage_path', 'imagestore', 'transient_store'):
    if actual['storage'].get(key) != original['storage'].get(key):
        raise SystemExit(f'Probe changed storage.{key}')
runner_temp = Path(os.environ['RUNNER_TEMP'])
github_env = Path(os.environ['GITHUB_ENV'])
if not runner_temp.is_absolute() or not runner_temp.is_dir() or not github_env.is_absolute():
    raise SystemExit('RUNNER_TEMP must exist and GITHUB_ENV must be absolute')
output = Path(tempfile.mkdtemp(prefix=f'issue92-{mode}-', dir=runner_temp))
config = output/'storage.conf'
shutil.copyfile(vendor, output/'storage.vendor.conf')
config.write_text(configured)
config.chmod(0o600)
(output/'selection.json').write_text(json.dumps({
    'mode': mode, 'graph_driver': 'overlay', 'mount_program': actual['storage']['options']['overlay']['mount_program'],
    'expected_graphroot': str(graphroot), 'expected_runroot': str(runroot),
    'configuration': str(config), 'selection_before_store_initialization': True,
}, indent=2)+'\n')
with github_env.open('a') as exported:
    exported.write(f'CONTAINERS_STORAGE_CONF={config}\n')
# The setup shell captures only this path and exports it before system migrate.
print(config)
