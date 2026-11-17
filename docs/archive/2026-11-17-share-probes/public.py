import json,os,pathlib,shutil,signal,subprocess as sp,tempfile,uuid
ROOT=pathlib.Path(__file__).resolve().parents[3];B=ROOT/'target/debug/pinfold';W=pathlib.Path(tempfile.mkdtemp(prefix='pfp.',dir='/tmp')).resolve();R=W/'repo';R.mkdir()
E={**os.environ,'XDG_CONFIG_HOME':str(W/'c'),'XDG_STATE_HOME':str(W/'s'),'PINFOLD_NO_UPDATE_CHECK':'1','GIT_CONFIG_GLOBAL':'/dev/null','GIT_CONFIG_NOSYSTEM':'1'}
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch();(R/'a').write_text('fixture\n')
for a in [['init','-q'],['add','.'],['-c','user.name=user','-c','user.email=user@fixture.invalid','commit','-qm','fixture']]:sp.run(['git','-C',str(R),*a],env=E,check=True)
def emit(s):print(str(s).replace(str(W),'$W').replace(str(ROOT),'/Users/user/project').replace(str(pathlib.Path.home()),'/Users/user'),flush=True)
def run(a):
 p=sp.run([str(B),*a],env=E,text=True,capture_output=True,timeout=90);emit('$ pinfold '+' '.join(a)+'\n'+p.stdout+p.stderr+f'rc={p.returncode}');return p
n='probe-public-'+uuid.uuid4().hex[:6];p=None
try:
 run(['--version']);spec={'name':n,'image':'localhost/pinfold/profile-default:latest','mounts':[{'host':str(R),'guest':'/project'}]};emit('spec '+json.dumps(spec))
 p=sp.Popen([str(B),'box','up'],stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE,env=E,text=True);p.stdin.write(json.dumps(spec)+'\n');p.stdin.flush();line=p.stdout.readline();emit(line.strip());assert json.loads(line)['event']=='ready'
 run(['box','exec',n,'--','sh','-c','id; cat /project/a; printf denied >> /project/a'])
 run(['box','exec',n,'--','git','-c','safe.directory=/project','-C','/project','log','-1','--format=%s'])
 emit('host bytes '+repr((R/'a').read_text()));run(['box','down',n]);out,err=p.communicate(timeout=90);emit(out+err+f'up rc={p.returncode}')
finally:
 if p is not None and p.poll() is None:
  p.send_signal(signal.SIGTERM);out,err=p.communicate(timeout=90);emit(out+err)
 run(['box','stat',n]);shutil.rmtree(W);emit('own public box and scratch removed=True; no disk created')
