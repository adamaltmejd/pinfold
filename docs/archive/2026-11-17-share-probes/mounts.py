import json,os,pathlib,shlex,shutil,subprocess as sp,tempfile,uuid
W=pathlib.Path(tempfile.mkdtemp(prefix='pfm.',dir='/tmp')).resolve();R=W/'repo';R.mkdir()
E={**os.environ,'GIT_CONFIG_GLOBAL':'/dev/null','GIT_CONFIG_NOSYSTEM':'1','GIT_AUTHOR_NAME':'user','GIT_AUTHOR_EMAIL':'user@fixture.invalid','GIT_COMMITTER_NAME':'user','GIT_COMMITTER_EMAIL':'user@fixture.invalid'}
I='localhost/pinfold/profile-default:latest';names=[]
def run(a):
 p=sp.run(list(map(str,a)),env=E,text=True,capture_output=True,timeout=90);row={'command':shlex.join(list(map(str,a))),'output':p.stdout+'\n'.join(l for l in p.stderr.splitlines() if not l.startswith('[')),'rc':p.returncode};print(json.dumps(row).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/Users/user'),flush=True);return p
def git(*a):sp.run(['git','-C',str(R),*a],env=E,capture_output=True,check=True)
git('init','-q','-b','main');(R/'a').write_text('base\n');git('add','.');git('commit','-qm','base')
(R/'.git/protected').write_text('protected\n');os.link(R/'.git/protected',R/'git-alias')
(W/'outside').write_text('outside\n');os.link(W/'outside',R/'outside-alias');(R/'outward').symlink_to(W/'unmounted');(W/'unmounted').write_text('unmounted-fixture\n')
(R/'.git/link').symlink_to('../a');(R/'bin').mkdir();(R/'bin/chmod').write_text('#!/bin/sh\necho SHARE_CHMOD uid=$(/usr/bin/id -u) cwd=$PWD args="$*"\n');(R/'bin/chmod').chmod(0o755)
def start(label,mounts,extra=()):
 n='probe-share-'+label+'-'+uuid.uuid4().hex[:6];names.append(n)
 a=['container','run','-d','--name',n,'--network','none','--read-only','--cap-drop','ALL','--kernel-arg','sysctl.user.max_user_namespaces=0','--user',f'{os.getuid()}:{os.getgid()}','--tmpfs','/tmp','-e','HOME=/tmp/home','-e','GIT_CONFIG_GLOBAL=/dev/null','-e','GIT_CONFIG_NOSYSTEM=1']
 for src,dst,ro in mounts:a+=['-v',f'{src}:{dst}'+(':ro' if ro else '')]
 p=run(a+list(extra)+['--entrypoint','/bin/sleep',I,'infinity']);assert p.returncode==0;return n
def x(n,c):return run(['container','exec','-w',R,n,'setpriv','--no-new-privs','sh','-c',c])
try:
 n=start('base',[(R,R,False),(R/'.git',R/'.git',True)])
 x(n,'id; grep -E "CapBnd|NoNewPrivs" /proc/self/status; cat /proc/sys/user/max_user_namespaces; git log -1 --format=%s; git status --short; printf ordinary >> a')
 x(n,'git -c safe.directory="$PWD" log -1 --format=%s');x(n,'git -c safe.directory="$PWD" status --short');x(n,'stat -c "%u:%g %n" . .git; grep virtiofs /proc/self/mountinfo')
 for c in ['printf x > .git','truncate -s 0 .git','printf x >> .git/protected','truncate -s 0 .git/protected','rm .git/protected','mv .git/protected .git/renamed','printf x > replacement; mv replacement .git/protected','rmdir .git','rm -rf .git','mv .git .git-old','mkdir other; mv -T other .git','mount -o remount,rw '+str(R/'.git'),'umount '+str(R/'.git'),'mount -t tmpfs tmpfs '+str(R/'.git'),'unshare -Urnm sh -c "mount -t tmpfs tmpfs '+str(R/'.git')+'"','cat outward','printf hardlink >> git-alias','printf allowed >> outside-alias','printf symlink >> .git/link']:x(n,c)
 print('host ordinary='+repr((R/'a').read_text())+' protected='+repr((R/'.git/protected').read_text())+' outside='+repr((W/'outside').read_text()),flush=True)
 for label,ms in [('reversed',[(R/'.git',R/'.git',True),(R,R,False)]),('obscured',[(R,R,False),(R/'.git',R/'.git',True),(R/'.git',R/'.git',False)]),('second-alias',[(R,R,False),(R/'.git',R/'.git',True),(R/'.git','/alias',False)])]:
  k=start(label,ms);x(k,'printf overlap >> '+('/alias/protected' if label=='second-alias' else '.git/protected'))
 for label,extra in [('default',[]),('path',['-e',f'PATH={R}/bin:/usr/bin:/bin'])]:
  k=start('root-'+label,[(R,R,False),(R/'.git',R/'.git',True)],extra)
  run(['container','exec','--user','0:0',k,'sh','-c','touch /tmp/harmless-fixture'])
  run(['container','exec','--user','0:0',k,'chmod','666','/tmp/harmless-fixture'])
 print('unmounted host fixture='+repr((W/'unmounted').read_text()))
finally:
 for n in names:run(['container','stop',n]);run(['container','delete',n])
 shutil.rmtree(W);print('own containers and fixture removed=True')
