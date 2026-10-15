"""Refuse an unintended storage mode on a disposable diagnostic runner."""
import json
import os
from pathlib import Path
import sys

info = json.loads(Path(sys.argv[1]).read_text())
store = info['store']
mode = os.environ['ISSUE92_STORAGE']
assert mode in ('native', 'fuse')
assert info['host']['security']['rootless'] is True
assert info['host']['cgroupManager'] == 'systemd'
assert store['graphDriverName'] == 'overlay'
data_home = Path(os.environ.get('XDG_DATA_HOME') or Path.home()/'.local/share')
assert Path(store['graphRoot']) == data_home/'containers/storage'
assert Path(store['runRoot']) == Path(f'/run/user/{os.getuid()}/containers')
assert store['transientStore'] is False
assert store['containerStore']['number'] == 0
assert store['imageStore']['number'] == 0
options = store['graphOptions']
assert options['overlay.mountopt'] == 'nodev'
mount_program = options.get('overlay.mount_program')
if isinstance(mount_program, dict):
    mount_program = mount_program.get('Executable')
if mode == 'native':
    assert not mount_program, 'Native cell fell back to a mount helper'
else:
    assert mount_program == '/usr/bin/fuse-overlayfs', 'FUSE helper was not selected'
print(f'Verified fresh rootless {mode} overlay store with systemd and nodev')
