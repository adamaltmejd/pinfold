#!/usr/bin/env python3
"""Manual fresh-home initialization check with a synthetic host tree."""
import json, os, pathlib, select, subprocess, tempfile, time

ROOT = pathlib.Path(__file__).resolve().parents[3]
P = ROOT/'target/debug/pinfold'
W = pathlib.Path(tempfile.mkdtemp(prefix='pf3.', dir='/tmp'))
E = {k:os.environ[k] for k in ('HOME','PATH','USER','LOGNAME','TMPDIR') if k in os.environ}
E.update(XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'),
         PINFOLD_NO_UPDATE_CHECK='1', FIXTURE='synthetic-parent-probe')
(W/'s/pinfold').mkdir(parents=True)
(W/'s/pinfold/maintenance').touch()
(W/'home/.pi/agent').mkdir(parents=True)
(W/'home/.pi/agent/settings.json').write_text('{"quietStartup":true}\n')

def emit(s):
    print(str(s).replace(str(ROOT), '/home/user/pinfold').replace(str(pathlib.Path.home()), '/home/user').replace(str(W),'$W'), flush=True)

def run(args):
    return subprocess.run([str(P),*args],env=E,capture_output=True,text=True,timeout=120)

def case(label, copied, routed):
    name = 'probe-pr3-'+W.name[4:]+'-'+label
    disk = name+'-home'
    attachment = {'name':disk,'guest':'/home/user'}
    if copied: attachment['from'] = str(W/'home')
    spec = {'name':name,'image':'localhost/pinfold/profile-default:latest','cpus':2,'memory':'1G',
            'env':{'HOME':'/home/user'},'disks':[attachment]}
    if routed:
        spec['egress'] = {'routes':{'model':{'login':'pi','provider':'anthropic','from':'FIXTURE','to':'http://127.0.0.1:9'}}}
    with tempfile.TemporaryFile(mode='w+t') as err:
        owner = subprocess.Popen([str(P),'box','up'],env=E,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,text=True)
        start = time.monotonic()
        try:
            owner.stdin.write(json.dumps(spec)+'\n'); owner.stdin.flush()
            if not select.select([owner.stdout],[],[],180)[0]: raise RuntimeError('readiness deadline')
            row = json.loads(owner.stdout.readline())
            emit(label+' '+json.dumps(row)+' seconds='+str(round(time.monotonic()-start,3)))
            if row['event'] == 'ready':
                result = run(['box','exec',name,'--','sh','-c',
                    'id -u; ls -ldn /home/user/.pi /home/user/.pi/agent; cat /home/user/.pi/agent/settings.json 2>/dev/null || true'])
                emit(result.stdout+result.stderr)
                emit(run(['box','down',name]).stdout)
            owner.stdin.close(); owner.wait(timeout=90)
            emit('owner-exit='+str(owner.returncode))
            err.seek(0); emit(err.read())
        finally:
            if owner.poll() is None:
                run(['box','down',name]); owner.terminate(); owner.wait(timeout=90)
            inventory = run(['disk','list','--json'])
            if any(json.loads(line)['name']==disk for line in inventory.stdout.splitlines() if line):
                removed = run(['disk','rm',disk]); emit('cleanup '+removed.stdout+removed.stderr)

case('copy-control', True, False)
case('route-control', False, True)
case('route-copy', True, True)
emit('owned probe boxes and disks removed')
