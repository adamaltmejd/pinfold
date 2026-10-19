import json,os,subprocess
from pathlib import Path
B=Path('/private/tmp/pinfold-native-acl-20261015');F=B/'fixture';M=F/'main';W=F/'wt';N='pf-native-acl-eight-20261015';rows=[]
def run(a):
 p=subprocess.run(list(map(str,a)),capture_output=True,text=True,timeout=40);return {'exit':p.returncode,'out':p.stdout,'err':p.stderr}
def need(a):
 r=run(a);assert r['exit']==0,(a,r);return r['out']
def guest(cmd):return run(['container','exec','--user','501:20','--workdir',W,N,'sh','-c',cmd])
P=W/'.git';ino=P.stat().st_ino
for p in W.iterdir():
 if not p.is_symlink() and p.is_file() and p.name!='.git' and p.stat().st_ino==ino:p.unlink()
assert P.stat().st_nlink==1
P.chmod(0o644);before=P.read_bytes();need(['chmod','+a','user:adam deny write,append,delete,writeattr,writeextattr,writesecurity,chown',P])
variants=['.git','.giT','.gIt','.gIT','.Git','.GiT','.GIt','.GIT'];args=[]
for name in variants:args+=['--read-only-path',str(W/name)]
rows.append({'name':'host_non_ascii_spelling_checks','aliases':{s:((W/s).exists() and (W/s).stat().st_ino==ino) for s in ['.gİt','.gıt','.ｇｉｔ']}})
try:
 need(['container','run','-d','--name',N,'--label','dev.research.owner=native-acl-20261015','--network','none','--cap-drop','ALL','--read-only','--tmpfs','/tmp','--cpus','2','--memory','512M','--user','501:20',*args,'-v',str(W)+':'+str(W),'-v',str(M/'.git')+':'+str(M/'.git')+':ro','pinfold/profile-default:latest','sleep','infinity'])
 for i,name in enumerate(variants):
  rows.append({'name':name,'chmod':guest('chmod 777 '+name),'write':guest('printf bad > '+name),'link':guest('ln '+name+' eight-link-'+str(i)),'host_mode':oct(P.stat().st_mode&0o777),'unchanged':P.read_bytes()==before})
 rows.append({'name':'pointer_symlink_chmod','result':guest('chmod 777 pointer-symlink'),'host_mode':oct(P.stat().st_mode&0o777)})
 rows.append({'name':'ordinary_positive','result':guest('printf ordinary > eight-ordinary; chmod 700 eight-ordinary; ln eight-ordinary eight-ordinary-link; mkdir eight-dir; printf replacement > eight-tmp; mv eight-tmp eight-ordinary; cat eight-ordinary')})
finally:
 rows.append({'name':'remove_box','result':run(['container','rm','-f',N])});rows.append({'name':'clear_acl','result':run(['chmod','-N',P])});(B/'eight-results.json').write_text(json.dumps(rows,indent=2));(B/'runtime-final.json').write_text(need(['container','list','--all','--format','json']))
print(json.dumps(rows,indent=2))
