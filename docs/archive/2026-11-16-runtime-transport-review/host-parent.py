#!/usr/bin/env python3
"""Linux real CLI parent trust check in a private user/mount namespace."""
import ctypes,os,subprocess as sp,sys
from pathlib import Path
binary=str(Path(sys.argv[1]).resolve()); uid,gid=os.getuid(),os.getgid()
c=ctypes.CDLL(None,use_errno=True)
assert c.unshare(0x10000000|0x20000)==0,ctypes.get_errno()
Path('/proc/self/setgroups').write_text('deny')
Path('/proc/self/uid_map').write_text(f'0 {uid} 1')
Path('/proc/self/gid_map').write_text(f'0 {gid} 1')
assert c.mount(None,b'/',None,16384|262144,None)==0,ctypes.get_errno()
assert c.mount(b'tmpfs',b'/tmp',b'tmpfs',2|4,b'mode=0777')==0,ctypes.get_errno()
w=Path('/tmp/fixture');(w/'s/pinfold').mkdir(parents=True);(w/'s/pinfold/maintenance').touch()
e={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
e.update(XDG_CONFIG_HOME=str(w/'c'),XDG_STATE_HOME=str(w/'s'),PINFOLD_NO_UPDATE_CHECK='1')
r=sp.run([binary,'disk','rm','probe-parent-absent','--json'],env=e,capture_output=True,timeout=15)
assert r.returncode==1 and b'runtime-path' in r.stderr and b'parent' in r.stderr,(r.returncode,r.stdout,r.stderr)
print('B root-owned parent0777 without sticky: real CLI refused runtime-path',flush=True)
os.chmod('/tmp',0o1777)
r=sp.run([binary,'disk','rm','probe-parent-absent','--json'],env=e,capture_output=True,timeout=15)
assert r.returncode==0,r.stderr
print('B same private parent corrected1777: real CLI succeeded; no host mounts or shared stores changed',flush=True)
