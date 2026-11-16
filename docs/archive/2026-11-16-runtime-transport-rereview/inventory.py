#!/usr/bin/env python3
"""Manual Linux namespace case. Run: python3 inventory.py BIN."""
import ctypes, os, subprocess as sp, sys
from pathlib import Path
binary=str(Path(sys.argv[1]).resolve()); uid,gid=os.getuid(),os.getgid()
c=ctypes.CDLL(None,use_errno=True)
assert c.unshare(0x10000000|0x20000)==0,ctypes.get_errno()
Path('/proc/self/setgroups').write_text('deny')
Path('/proc/self/uid_map').write_text(f'0 {uid} 1')
Path('/proc/self/gid_map').write_text(f'0 {gid} 1')
assert c.mount(None,b'/',None,16384|262144,None)==0,ctypes.get_errno()
assert c.mount(b'tmpfs',b'/tmp',b'tmpfs',2|4,b'mode=1777')==0,ctypes.get_errno()
w=Path('/tmp/probe-inventory');(w/'s/pinfold').mkdir(parents=True)
(w/'s/pinfold/maintenance').touch();h=Path('/tmp/pinfold-0')
e={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
e.update(XDG_CONFIG_HOME=str(w/'c'),XDG_STATE_HOME=str(w/'s'),PINFOLD_NO_UPDATE_CHECK='1')
for args in (['doctor'],['clean','--dry-run']):
    assert not h.exists()
    r=sp.run([binary]+args,env=e,cwd=w,capture_output=True,timeout=90)
    print('3 command='+repr(args)+' exit='+str(r.returncode)+' host_directory_absent='+str(not h.exists()),flush=True)
    assert r.returncode==0 and not h.exists(),(r.returncode,r.stdout,r.stderr)
r=sp.run([binary,'disk','rm','probe-inventory-absent','--json'],env=e,cwd=w,capture_output=True,timeout=30)
assert r.returncode==0 and h.is_dir() and h.stat().st_mode&0o777==0o700,(r.returncode,r.stdout,r.stderr)
print('3 mutation control: absent disk rm succeeded and created checked H mode0700; private namespace only',flush=True)
