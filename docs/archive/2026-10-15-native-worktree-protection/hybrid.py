import json,os,subprocess,time
from pathlib import Path
B=Path('/private/tmp/pinfold-native-acl-20261015');F=B/'fixture';M=F/'main';W=F/'wt';H=W/'.hooks';hook=H/'pre-commit';N='pf-native-acl-hybrid-20261015';rows=[];acl=[]
env=dict(os.environ,GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1')
def run(a,**kw):
 p=subprocess.run(list(map(str,a)),capture_output=True,text=True,timeout=40,**kw);return {'exit':p.returncode,'out':p.stdout,'err':p.stderr}
def need(a,**kw):
 r=run(a,**kw);assert r['exit']==0,(a,r);return r['out']
def guest(cmd):return run(['container','exec','--user','501:20','--workdir',W,N,'sh','-c',cmd])
def protect(p,rights):need(['chmod','+a','user:adam deny '+rights,p]);acl.append(p)
fr='write,append,delete,writeattr,writeextattr,writesecurity,chown';dr='add_file,add_subdirectory,delete,delete_child,writeattr,writeextattr,writesecurity,chown'
protect(W/'.git',fr);protect(H,dr);protect(hook,fr);hook.chmod(0o644);pointer=(W/'.git').read_bytes()
# Deliberately unsafe fixture: admission must reject this existing writable alias.
alias=W/'preexisting-hook-alias';os.link(hook,alias)
try:
 need(['container','run','-d','--name',N,'--label','dev.research.owner=native-acl-20261015','--network','none','--cap-drop','ALL','--read-only','--tmpfs','/tmp','--cpus','2','--memory','512M','--user','501:20','--read-only-path',str(H),'--read-only-path',str(W/'.git'),'-v',str(W)+':'+str(W),'-v',str(M/'.git')+':'+str(M/'.git')+':ro','-v',str(B/'tools')+':/tools:ro','pinfold/profile-default:latest','sleep','infinity'])
 rows.append({'name':'directory_alias_first_chmod','result':guest('chmod +x .HOOKS/pre-commit'),'mode':oct(hook.stat().st_mode&0o777)})
 rows.append({'name':'new_hook_hardlink','result':guest('ln .HOOKS/pre-commit new-hook-alias')})
 for p in ['.git','.giT','.gIt','.gIT','.Git','.GiT','.GIt','.GIT']:
  rows.append({'name':'pointer_case_'+p,'result':guest('printf bad > '+p),'unchanged':pointer==(W/'.git').read_bytes()})
 rows.append({'name':'pointer_alias_chmod','result':guest('chmod 777 .GIT'),'pointer_bytes_unchanged':pointer==(W/'.git').read_bytes(),'mode':oct((W/'.git').stat().st_mode&0o777)})
 rows.append({'name':'preexisting_hook_alias_chmod','result':guest('chmod +x preexisting-hook-alias'),'mode':oct(hook.stat().st_mode&0o777),'acl':run(['ls','-le',hook])})
 # A fixed harmless marker is the only hook payload in this disposable repository.
 marker=F/'hook-marker';rows.append({'name':'host_fixture_commit','result':run(['git','-C',W,'commit','--allow-empty','-qm','hybrid fixture'],env=env),'marker_exists':marker.exists(),'marker':marker.read_text() if marker.exists() else None})
 expected=need(['git','-C',W,'rev-parse','HEAD'],env=env).strip();t=time.monotonic();samples=[]
 # Measure native cache visibility; each observation is retained, not a retrying pass/fail test.
 while time.monotonic()-t<4:
  r=guest('git -c safe.directory="'+str(W)+'" rev-parse HEAD');samples.append({'seconds':time.monotonic()-t,**r})
  if r['exit']==0 and r['out'].strip()==expected:break
 rows.append({'name':'host_commit_visibility_measurement','expected':expected,'samples':samples})
finally:
 rows.append({'name':'remove_box','result':run(['container','rm','-f',N])})
 for p in reversed(acl):
  if p.exists():rows.append({'name':'clear_fixture_acl','path':str(p),'result':run(['chmod','-N',p])})
 (B/'hybrid-results.json').write_text(json.dumps(rows,indent=2));(B/'runtime-final.json').write_text(need(['container','list','--all','--format','json']))
print(json.dumps([r for r in rows if r['name']!='clear_fixture_acl'],indent=2))
