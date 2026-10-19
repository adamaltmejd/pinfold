import json,os,subprocess
from pathlib import Path
B=Path('/private/tmp/pinfold-native-acl-20261015');F=B/'fixture';M=F/'main';W=F/'wt';N='pf-native-acl-eight-noacl-20261015';rows=[]
def run(a):
 p=subprocess.run(list(map(str,a)),capture_output=True,text=True,timeout=40);return {'exit':p.returncode,'out':p.stdout,'err':p.stderr}
def need(a):
 r=run(a);assert r['exit']==0,(a,r);return r['out']
def guest(cmd):return run(['container','exec','--user','501:20','--workdir',W,N,'sh','-c',cmd])
P=W/'.git';ino=P.stat().st_ino
for p in W.iterdir():
 if not p.is_symlink() and p.is_file() and p.name!='.git' and p.stat().st_ino==ino:p.unlink()
assert P.stat().st_nlink==1
P.chmod(0o644);before=P.read_bytes()
variants=['.git','.giT','.gIt','.gIT','.Git','.GiT','.GIt','.GIT'];args=[]
for name in variants:args+=['--read-only-path',str(W/name)]
rows.append({'name':'host_non_ascii_spelling_checks','aliases':{s:((W/s).exists() and (W/s).stat().st_ino==ino) for s in ['.gİt','.gıt','.ｇｉｔ']}})
try:
 need(['container','run','-d','--name',N,'--label','dev.research.owner=native-acl-20261015','--network','none','--cap-drop','ALL','--read-only','--tmpfs','/tmp','--cpus','2','--memory','512M','--user','501:20',*args,'--read-only-path',str(W/'deep/middle/hooks'),'-v',str(W)+':'+str(W),'-v',str(M/'.git')+':'+str(M/'.git')+':ro','pinfold/profile-default:latest','sleep','infinity'])
 for i,name in enumerate(variants):
  rows.append({'name':name,'chmod':guest('chmod 777 '+name),'write':guest('printf bad > '+name),'link':guest('ln '+name+' eight-link-'+str(i)),'host_mode':oct(P.stat().st_mode&0o777),'unchanged':P.read_bytes()==before})
 for name in variants:
  rows.append({'name':'namespace_'+name,'unlink':guest('rm '+name),'rename':guest('mv '+name+' eight-moved'),'replace':guest('printf replacement > eight-replace; mv -f eight-replace '+name),'unchanged':P.read_bytes()==before})
 rows.append({'name':'pointer_symlink_chmod','result':guest('chmod 777 pointer-symlink'),'host_mode':oct(P.stat().st_mode&0o777)})
 rows.append({'name':'ordinary_positive','result':guest('set -e; printf ordinary > noacl-ordinary; chmod 700 noacl-ordinary; ln noacl-ordinary noacl-ordinary-link; mkdir noacl-dir; printf replacement > noacl-tmp; mv noacl-tmp noacl-ordinary; cat noacl-ordinary')})
 rows.append({'name':'nested_ancestor_without_acl','result':guest('mv deep renamed-deep'),'host_renamed':(W/'renamed-deep').exists()})
finally:
 rows.append({'name':'remove_box','result':run(['container','rm','-f',N])});rows.append({'name':'clear_acl','result':run(['chmod','-N',P])});(B/'eight-noacl-results.json').write_text(json.dumps(rows,indent=2));(B/'runtime-final.json').write_text(need(['container','list','--all','--format','json']))
if (W/'renamed-deep').exists():(W/'renamed-deep').rename(W/'deep')
print(json.dumps(rows,indent=2))
