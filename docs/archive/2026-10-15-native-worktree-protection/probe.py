import hashlib,json,os,shutil,subprocess,time
from pathlib import Path
B=Path('/private/tmp/pinfold-native-acl-20261015');F=B/'fixture';M=F/'main';W=F/'wt';N='pf-native-acl-20261015';rows=[];acl=[]
def run(a,**kw):
 p=subprocess.run(list(map(str,a)),capture_output=True,text=True,timeout=40,**kw);return {'exit':p.returncode,'out':p.stdout,'err':p.stderr}
def need(a,**kw):
 r=run(a,**kw);assert r['exit']==0,(a,r);return r['out']
env=dict(os.environ,GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1')
def git(p,*a):return need(['git','-C',p,*a],env=env)
def protect(p,rights):
 need(['chmod','+a','user:adam deny '+rights,p]);acl.append(p)
def guest(cmd,uid='501:20'):return run(['container','exec','--user',uid,'--workdir',W,N,'sh','-c',cmd])
def snapshot():return {'bytes':(W/'.git').read_text() if (W/'.git').exists() else None,'acl':run(['ls','-led',W/'.git'])}
def attempt(label,cmd,uid='501:20'):
 before=snapshot();r=guest(cmd,uid);after=snapshot();rows.append({'name':label,'uid':uid,'result':r,'before':before,'after':after});return r
assert not F.exists();M.mkdir(parents=True);(B/'tools').mkdir();shutil.copy2(B/'xattr_probe',B/'tools/xattr_probe')
git(M,'init','-q');git(M,'config','user.name','Fixture');git(M,'config','user.email','fixture@example.invalid');git(M,'config','commit.gpgsign','false');(M/'source').write_text('base\n');git(M,'add','.');git(M,'commit','-qm','base');git(M,'worktree','add','-qb','work',W)
W.chmod(0o777);(W/'source').chmod(0o666);(W/'.git').chmod(0o666);pointer=(W/'.git').read_text();os.link(W/'.git',W/'pointer-hardlink');(W/'pointer-symlink').symlink_to('.git')
file_rights='write,append,delete,writeattr,writeextattr,writesecurity,chown'
protect(W/'.git',file_rights)
# The fixture owner can remove its ACL, even when writesecurity is denied.
c=W/'cleanup-control';c.write_text('safe');protect(c,file_rights);rows.append({'name':'native_owner_can_remove_acl','result':run(['chmod','-N',c])});assert '+' not in need(['ls','-ld',c]).split()[0]
for principal in ['adam','nobody']:
 p=W/('deny-'+principal);p.write_text('safe');p.chmod(0o666);need(['chmod','+a','user:'+principal+' deny write,append',p]);acl.append(p)
D=W/'.vscode';D.mkdir();(D/'settings.json').write_text('trusted');protect(D,'add_file,add_subdirectory,delete,delete_child,writeattr,writeextattr,writesecurity,chown');protect(D/'settings.json',file_rights)
(B/'runtime-before.json').write_text(need(['container','list','--all','--format','json']));(B/'image.json').write_text(need(['container','image','inspect','pinfold/profile-default:latest']))
try:
 need(['container','run','-d','--name',N,'--label','dev.research.owner=native-acl-20261015','--network','none','--cap-drop','ALL','--read-only','--tmpfs','/tmp','--cpus','2','--memory','512M','--user','501:20','-v',str(W)+':'+str(W),'-v',str(M/'.git')+':'+str(M/'.git')+':ro','-v',str(B/'tools')+':/tools:ro','pinfold/profile-default:latest','sleep','infinity'])
 ps=need(['ps','-axo','pid,ppid,uid,comm']);(B/'processes.txt').write_text('\n'.join(l for l in ps.splitlines() if 'container-' in l or 'Virtualization.VirtualMachine' in l)+'\n')
 rows.append({'name':'host_pointer_write','result':run(['sh','-c','printf bad > "$1"','probe',W/'.git'])})
 for uid in ['501:20','0:0','12345:12345']:
  rows.append({'name':'identity_controls','uid':uid,'result':guest('id; printf allowed > ordinary-'+uid.split(':')[0]+'; stat -c "%u:%g %a" ordinary-'+uid.split(':')[0]+'; printf bad > deny-adam; printf other > deny-nobody',uid)})
  for path in ['.git','.GIT','pointer-hardlink','pointer-symlink']:attempt('write_'+path,'printf bad > '+path,uid)
  attempt('metadata_strip_attempt','/tools/xattr_probe .GIT',uid)
 for label,cmd in [('unlink','rm .GIT'),('rename','mv .GIT pointer-moved'),('replace','printf replace > tmp-pointer; mv -f tmp-pointer .GIT'),('hardlink_create','ln .git new-pointer-alias'),('editor_create','printf bad > .VSCODE/new'),('editor_replace','printf bad > tmp-config; mv -f tmp-config .VSCODE/settings.json'),('editor_rename','mv .VSCODE editor-moved')]:attempt(label,cmd)
 rows.append({'name':'ordinary_atomic_edit','result':guest('printf edited > temp; mv temp source; mkdir newdir; printf child > newdir/file')})
 rows.append({'name':'guest_git_reads','result':guest('git -c safe.directory="'+str(W)+'" status --short; git -c safe.directory="'+str(W)+'" log -1 --oneline')})
 rows.append({'name':'guest_external_metadata_write','result':guest('printf bad > "'+str(M/'.git/config')+'"')})
 rows.append({'name':'host_commit','result':run(['git','-C',W,'-c','core.hooksPath=/dev/null','add','source'],env=env)})
 rows.append({'name':'host_commit_result','result':run(['git','-C',W,'-c','core.hooksPath=/dev/null','commit','-qm','host edit'],env=env)})
 rows.append({'name':'guest_after_host_commit','result':guest('git -c safe.directory="'+str(W)+'" log -1 --format=%s')})
 rows.append({'name':'host_new_files_ownership','result':run(['ls','-ln',W/'ordinary-0',W/'ordinary-501',W/'ordinary-12345'])})
 assert (W/'.git').read_text()==pointer,'pointer changed'
finally:
 rows.append({'name':'remove_box','result':run(['container','rm','-f',N])})
 for p in reversed(acl):
  if p.exists():rows.append({'name':'clear_fixture_acl','path':str(p),'result':run(['chmod','-N',p])})
 (B/'results.json').write_text(json.dumps(rows,indent=2));(B/'runtime-after.json').write_text(need(['container','list','--all','--format','json']))
print(json.dumps([{'name':r['name'],'uid':r.get('uid'),'result':r.get('result')} for r in rows if r['name']!='clear_fixture_acl'],indent=2))
