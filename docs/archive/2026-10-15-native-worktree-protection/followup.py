import json,os,subprocess
from pathlib import Path
B=Path('/private/tmp/pinfold-native-acl-20261015');F=B/'fixture';M=F/'main';W=F/'wt';N='pf-native-acl-followup-20261015';rows=[];acl=[]
env=dict(os.environ,GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1')
def run(a,**kw):
 p=subprocess.run(list(map(str,a)),capture_output=True,text=True,timeout=40,**kw);return {'exit':p.returncode,'out':p.stdout,'err':p.stderr}
def need(a,**kw):
 r=run(a,**kw);assert r['exit']==0,(a,r);return r['out']
def guest(cmd):return run(['container','exec','--user','501:20','--workdir',W,N,'sh','-c',cmd])
def protect(p,rights):need(['chmod','+a','user:adam deny '+rights,p]);acl.append(p)
file_rights='write,append,delete,writeattr,writeextattr,writesecurity,chown';dir_rights='add_file,add_subdirectory,delete,delete_child,writeattr,writeextattr,writesecurity,chown'
pointer=(W/'.git').read_bytes();protect(W/'.git',file_rights)
H=W/'.hooks';H.mkdir();hook=H/'pre-commit';marker=F/'hook-marker';hook.write_text('#!/bin/sh\nprintf executed > '+str(marker)+'\n');hook.chmod(0o644);protect(hook,file_rights);protect(H,dir_rights)
need(['git','-C',W,'config','core.hooksPath',H],env=env)
control=W/'acl-set-control';control.write_text('ordinary');control.chmod(0o666)
for p in [W/'deep',W/'deep/middle',W/'deep/middle/hooks']:p.mkdir()
protect(W/'deep','delete');protect(W/'deep/middle','delete');protect(W/'deep/middle/hooks',dir_rights)
held=W/'held-open';held.write_text('original')
try:
 need(['container','run','-d','--name',N,'--label','dev.research.owner=native-acl-20261015','--network','none','--cap-drop','ALL','--read-only','--tmpfs','/tmp','--cpus','2','--memory','512M','--user','501:20','--read-only-path',str(H),'-v',str(W)+':'+str(W),'-v',str(M/'.git')+':'+str(M/'.git')+':ro','-v',str(B/'tools')+':/tools:ro','pinfold/profile-default:latest','sleep','infinity'])
 rows.append({'name':'git_on_fresh_mount','result':guest('git -c safe.directory="'+str(W)+'" log -1 --format=%s')})
 rows.append({'name':'valid_posix_acl_set_control','result':guest('/tools/acl_set_probe acl-set-control'),'host_acl':run(['ls','-le',control])})
 rows.append({'name':'valid_posix_acl_set_pointer','result':guest('/tools/acl_set_probe .GIT'),'host_acl':run(['ls','-le',W/'.git']),'pointer_unchanged':pointer==(W/'.git').read_bytes()})
 rows.append({'name':'new_hardlink_write','result':guest('ln .git new-alias-followup; printf bad > new-alias-followup'),'pointer_unchanged':pointer==(W/'.git').read_bytes()})
 for name,cmd in [('ancestor1','mv DEEP deep-moved'),('ancestor2','mv DEEP/MIDDLE DEEP/middle-moved'),('ancestor_ordinary_child','printf allowed > deep/source')]:rows.append({'name':name,'result':guest(cmd)})
 rows.append({'name':'host_commit_disabled_hook','result':run(['git','-C',W,'commit','--allow-empty','-qm','disabled hook'],env=env),'marker_exists':marker.exists()})
 rows.append({'name':'literal_hook_chmod','result':guest('chmod +x .hooks/pre-commit'),'host_mode':oct(hook.stat().st_mode&0o777)})
 rows.append({'name':'alias_hook_chmod','result':guest('chmod +x .HOOKS/pre-commit'),'host_mode':oct(hook.stat().st_mode&0o777),'host_acl':run(['ls','-le',hook])})
 rows.append({'name':'host_commit_after_guest_chmod','result':run(['git','-C',W,'commit','--allow-empty','-qm','after chmod'],env=env),'marker_exists':marker.exists(),'marker':marker.read_text() if marker.exists() else None})
 # Demonstrate that applying an ACL after granting a writable descriptor is too late.
 p=subprocess.Popen(['container','exec','-i','--user','501:20','--workdir',str(W),N,'sh','-c','exec 3>held-open; printf READY; read gate; printf late >&3'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
 assert p.stdout.read(5)=='READY';protect(held,file_rights);p.stdin.write('go\n');p.stdin.flush();p.stdin.close();p.wait(timeout=10)
 rows.append({'name':'preopened_descriptor','exit':p.returncode,'out':p.stdout.read(),'err':p.stderr.read(),'host_bytes':held.read_text()})
 rows.append({'name':'later_git_visibility','result':guest('git -c safe.directory="'+str(W)+'" log -1 --format=%s')})
finally:
 rows.append({'name':'remove_box','result':run(['container','rm','-f',N])})
 for p in reversed(acl):
  if p.exists():rows.append({'name':'clear_fixture_acl','path':str(p),'result':run(['chmod','-N',p])})
 (B/'followup-results.json').write_text(json.dumps(rows,indent=2));(B/'runtime-final.json').write_text(need(['container','list','--all','--format','json']))
print(json.dumps([r for r in rows if r['name']!='clear_fixture_acl'],indent=2))
