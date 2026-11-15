#!/usr/bin/env python3
"""Manual CLI acceptance: generated mounts, disk population and recovery."""
import copy, json, os, pathlib, select, subprocess, sys, tempfile
ROOT = pathlib.Path(__file__).resolve().parents[3]
P = pathlib.Path(sys.argv[1] if len(sys.argv)>1 else ROOT/'target/debug/pinfold').resolve()
IMAGE = os.environ.get('IMAGE', 'localhost/pinfold/profile-default:latest')
W = pathlib.Path(tempfile.mkdtemp(prefix='pf3.', dir='/tmp'))
E = {k:os.environ[k] for k in ('HOME','PATH','USER','LOGNAME','TMPDIR','XDG_CACHE_HOME') if k in os.environ}
E.update(XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'), PINFOLD_NO_UPDATE_CHECK='1',
         FIXTURE_TOKEN='synthetic-validation-token', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch()
F = W/'home'; (F/'.pi/agent/extensions').mkdir(parents=True)
(F/'.pi/agent/settings.json').write_text('{"quietStartup":true}\n')
(F/'.pi/agent/extensions/user.ts').write_text('export default function() {}\n')
owners, disks, serial = {}, set(), 0
def emit(s):
    print(str(s).replace(str(ROOT),'/home/user/pinfold').replace(str(pathlib.Path.home()),'/home/user').replace(str(W),'$W'), flush=True)
def run(args, required=True):
    p = subprocess.run([str(P),*args], env=E, capture_output=True, text=True, timeout=120)
    if required and p.returncode: raise RuntimeError(p.stdout+p.stderr)
    return p
def base():
    global serial
    serial += 1
    name = 'probe-pr3-v-'+W.name[4:]+'-'+str(serial)
    return {'name':name,'image':IMAGE,'cpus':2,'memory':'1G','env':{'HOME':'/home/user'},
            'disks':[{'name':name+'-home','guest':'/home/user'}],
            'egress':{'allow':[],'routes':{'model':{'login':'pi','provider':'anthropic','from':'FIXTURE_TOKEN','to':'http://127.0.0.1:9'}}}}
def up(spec, event='ready', reason=None):
    err = tempfile.TemporaryFile(mode='w+t')
    p = subprocess.Popen([str(P),'box','up'],env=E,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,text=True)
    owners[spec['name']] = (p,err); disks.update(d['name'] for d in spec['disks'])
    p.stdin.write(json.dumps(spec)+'\n'); p.stdin.flush()
    if not select.select([p.stdout],[],[],180)[0]: raise RuntimeError('readiness deadline')
    line = p.stdout.readline(); row = json.loads(line)
    if row.get('event') != event or reason and row.get('reason') != reason:
        err.seek(0); raise RuntimeError(line+err.read())
    if event != 'ready':
        p.stdin.close(); p.wait(timeout=90); err.seek(0)
        if reason != 'spec': emit('observed '+event+' reason='+str(row.get('reason'))+' detail='+row.get('detail',''))
        err.close(); del owners[spec['name']]
    return row
def down(spec):
    name = spec['name']
    if name in owners:
        run(['box','down',name]); p,err = owners.pop(name)
        p.stdin.close(); p.wait(timeout=90); err.close()
def x(spec, command):
    return run(['box','exec',spec['name'],'--','sh','-c','set -eu; '+command]).stdout.strip()
def remove_disk(spec):
    inventory = run(['disk','list','--json']).stdout
    names = {json.loads(line)['name'] for line in inventory.splitlines() if line}
    for disk in spec['disks']:
        if disk['name'] in names: run(['disk','rm',disk['name']])
        disks.discard(disk['name'])
def cycle(spec):
    up(spec); assert x(spec,'true') == ''; down(spec); remove_disk(spec)
try:
    for label,value in [('missing',None),('relative','relative'),('outside','/outside'),('comma','/home/user/invalid,path')]:
        s = base()
        if value is None: del s['env']['HOME']
        else: s['env']['HOME'] = value
        up(s,'refused','spec'); s['env']['HOME']='/home/user'; cycle(s)
        emit('HOME '+label+' refusal(spec) and corrected writable HOME passed')
    s=base(); s['disks'][0]['readonly']=True
    up(s,'refused','spec'); s['disks'][0]['readonly']=False; cycle(s)
    emit('HOME readonly refusal(spec) and writable correction passed')
    for guest in ['/etc/claude-code','/etc/claude-code/child','/etc/a/../claude-code',
                  '/etc','/home/user/.pi/agent/extensions','/home/user/.pi/agent/extensions/pinfold-login/child']:
        s=base(); s['egress']['routes']['claude']={'login':'claude','from':'FIXTURE_TOKEN','to':'http://127.0.0.1:9'}
        s['mounts']=[{'host':str(F),'guest':guest}]
        up(s,'refused','spec'); s['mounts'][0]['guest']='/fixture'; cycle(s)
        emit('generated collision '+guest+' refusal(spec) and corrected mount passed')
    s=base(); s['harnesses']=['pi']; up(s,'refused','spec'); del s['harnesses']; up(s)
    legacy=run(['box','exec',s['name'],'--harness','pi','--','true'],False)
    assert legacy.returncode == 1 and '--harness' in legacy.stderr
    emit('legacy flag exit=1 diagnostic='+legacy.stderr.splitlines()[0])
    assert x(s,'printf corrected') == 'corrected'
    emit('legacy key and flag refused; corrected spec and command passed')
    down(s); remove_disk(s)
    s=base(); s['disks'][0]['from']=str(F); up(s)
    assert x(s,'cat "$HOME/.pi/agent/settings.json"') == '{"quietStartup":true}'
    expected=x(s,'stat -c "%u:%g:%a" "$HOME/.pi" "$HOME/.pi/agent"')
    assert all(row.startswith(str(os.getuid())+':') for row in expected.splitlines())
    assert x(s,'cat "$HOME/.pi/agent/extensions/user.ts"') == 'export default function() {}'
    x(s,'printf retained > "$HOME/retained"; chmod 700 "$HOME/.pi"')
    before=x(s,'stat -c "%u:%g:%a" "$HOME/.pi" "$HOME/.pi/agent"')
    down(s); (F/'.pi/agent/settings.json').write_text('{"changedHostFixture":true}\n'); up(s)
    assert x(s,'cat "$HOME/.pi/agent/settings.json"') == '{"quietStartup":true}'
    assert x(s,'stat -c "%u:%g:%a" "$HOME/.pi" "$HOME/.pi/agent"') == before
    emit('fresh from containing .pi/settings plus Pi route passed; reused home not repopulated; ownership/mode retained')
    down(s)
    plain=copy.deepcopy(s); del plain['egress']
    for kind in ('symlink','file'):
        up(plain)
        x(plain,'mkdir -p "$HOME/outside"; printf sentinel > "$HOME/outside/sentinel"; mv "$HOME/.pi/agent/extensions" "$HOME/.pi/agent/saved-extensions"')
        x(plain,'ln -s "$HOME/outside" "$HOME/.pi/agent/extensions"' if kind=='symlink' else 'printf blocked > "$HOME/.pi/agent/extensions"')
        down(plain); failed=up(s,'failed')
        assert any(v in failed['detail'] for v in ('Not a directory','ELOOP','ENOTDIR','symlink'))
        readonly=copy.deepcopy(plain); readonly['disks'][0]['readonly']=True; readonly['disks'][0].pop('from',None)
        up(readonly,'refused','disk-unclean')
        up(plain)
        assert x(plain,'find "$HOME/outside" -mindepth 1 -maxdepth 1 -printf "%f\\n"') == 'sentinel'
        assert x(plain,'cat "$HOME/.pi/agent/settings.json"; cat "$HOME/retained"') == '{"quietStartup":true}\nretained'
        assert x(plain,'stat -c "%u:%g:%a" "$HOME/.pi" "$HOME/.pi/agent"') == before
        x(plain,'rm "$HOME/.pi/agent/extensions"; mv "$HOME/.pi/agent/saved-extensions" "$HOME/.pi/agent/extensions"')
        down(plain); up(s)
        assert x(s,'cat "$HOME/.pi/agent/extensions/user.ts"') == 'export default function() {}'
        down(s); emit('reused '+kind+' refused without target writes; unclean refusal, writable recovery and same-home correction passed')
    remove_disk(s)
    if sys.platform.startswith('linux'):
        s=base(); s['egress']['routes']['claude']={'login':'claude','from':'FIXTURE_TOKEN','to':'http://127.0.0.1:9'}
        s['mounts']=[{'host':str(F),'guest':'/fixture-'+str(i)} for i in range(22)]
        up(s,'refused','spec'); s['mounts'].pop(); cycle(s)
        emit('Linux attachment 25 refused; attachment 24 (21 caller + home + two generated) passed')
    parent=base(); parent.pop('egress'); parent['env']['HOME']='/state'; parent['disks'][0]['guest']='/state'
    up(parent); x(parent,'mkdir -p /state/home'); down(parent)
    child=base(); child['env']['HOME']='/state/home'; child['disks'][0]['guest']='/state/home'
    parent['disks'][0]['readonly']=True; child['disks'].insert(0,parent['disks'][0])
    up(child); assert x(child,'test -r "$HOME/.pi/agent/extensions/pinfold-login/index.ts"; printf writable > "$HOME/ok"') == ''
    down(child); remove_disk(child); emit('readonly ancestor with deeper writable HOME backing passed')
    emit('validation cases passed')
finally:
    for name,(p,err) in list(owners.items()):
        run(['box','down',name],False)
        if not p.stdin.closed: p.stdin.close()
        try: p.wait(timeout=90)
        except subprocess.TimeoutExpired: p.terminate(); p.wait(timeout=30)
        err.close()
    names={json.loads(line)['name'] for line in run(['disk','list','--json']).stdout.splitlines() if line}
    for disk in disks & names: run(['disk','rm',disk])
    emit('validation owned boxes/disks removed')
