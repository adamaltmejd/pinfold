#!/usr/bin/env python3
"""Manual managed-cache acceptance. P=... PROBE_ROOT=/tmp/pfb.XXXX cache.py MODE."""
import hashlib, json, os, pathlib, re, select, shutil, subprocess, sys, tempfile, time
ROOT=pathlib.Path(__file__).resolve().parents[3]; LINUX=sys.platform.startswith('linux')
P=pathlib.Path(os.environ['P']).resolve(); MODE=sys.argv[1]
W=pathlib.Path(os.environ.get('PROBE_ROOT') or tempfile.mkdtemp(prefix='pfb.',dir='/tmp')).resolve()
if not str(W).startswith(('/tmp/pfb.','/private/tmp/pfb.')) or len(str(W))>35: raise RuntimeError('use short dedicated /tmp/pfb. root')
E={k:os.environ[k] for k in ('HOME','PATH','USER','LOGNAME','TMPDIR','XDG_CACHE_HOME') if k in os.environ}
E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true')
(W/'s/pinfold').mkdir(parents=True,exist_ok=True); (W/'s/pinfold/maintenance').touch()
serial=0
def emit(s): print(str(s).replace(str(ROOT),'/home/user/pinfold').replace(str(pathlib.Path.home()),'/home/user').replace(str(W),'$W'),flush=True)
def check(ok,label):
    if not ok: raise RuntimeError(label)
def run(args,cwd=None,required=True):
    p=subprocess.run(list(map(str,args)),env=E,cwd=cwd or W,text=True,capture_output=True,timeout=1500)
    if required and p.returncode: emit((p.stdout+p.stderr)[-3000:]); raise RuntimeError('command failed')
    return p
def identity(ref):
    if LINUX:
        catalog=pathlib.Path(E.get('XDG_CACHE_HOME',str(pathlib.Path.home()/'.cache')))/'pinfold/images/catalog.json'
        return json.loads(catalog.read_text())['references'].get(ref) if catalog.exists() else None
    rows=json.loads(run(['container','image','list','--format','json']).stdout)
    return next((r['id'] for r in rows if r['configuration']['name']==ref),None)
def backing():
    if LINUX: return None
    path=pathlib.Path.home()/'Library/Application Support/com.apple.container/containers/buildkit'
    return int(run(['du','-sk',path]).stdout.split()[0])*1024
def build(args,cwd=None,required=True):
    global serial
    free=shutil.disk_usage('/System/Volumes/Data' if not LINUX else '/').free
    emit('build-start-free-bytes='+str(free))
    check(LINUX or free>=12*1024**3,'under 12 GiB free; build not started')
    serial+=1; start=time.monotonic(); p=run([P,'build',*args],cwd,False)
    (W/(MODE+'-'+str(serial)+'.raw')).write_text(p.stdout+p.stderr)
    rows=[]
    for log in re.findall(r'egress log (\S+)',p.stderr): rows.extend(json.loads(l) for l in pathlib.Path(log).read_text().splitlines())
    discovery=sum(r.get('host') in ('api.github.com','downloads.claude.ai') for r in rows)
    assets=sum(r.get('host') in ('github.com','release-assets.githubusercontent.com') for r in rows)
    apt=sum(r.get('host')=='deb.debian.org' for r in rows)
    emit('managed-build args='+json.dumps(args)+' exit='+str(p.returncode)+' seconds='+str(round(time.monotonic()-start,3))+' audit-discovery='+str(discovery)+' audit-assets='+str(assets)+' audit-debian='+str(apt))
    if required and p.returncode: emit((p.stdout+p.stderr)[-3000:]); raise RuntimeError('managed build failed')
    return p,rows
def observe(ref):
    name='probe-cache-'+W.name[4:]; err=tempfile.TemporaryFile(mode='w+t')
    p=subprocess.Popen([str(P),'box','up'],env=E,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,text=True)
    try:
        p.stdin.write(json.dumps({'name':name,'image':ref,'cpus':2,'memory':'1G','env':{'HOME':'/home/user'},'disks':[{'name':name+'-home','guest':'/home/user'}]})+'\n'); p.stdin.flush()
        check(select.select([p.stdout],[],[],180)[0],'image readiness deadline'); row=json.loads(p.stdout.readline())
        check(row.get('event')=='ready','built image launches')
        value=run([P,'box','exec',name,'--','sh','-c','set -eu; pi --version; claude --version; codex --version; cat /opt/pinfold/agents/install.txt']).stdout
        emit('image='+str(identity(ref))+' versions='+json.dumps(value.splitlines()[:3])+' receipt-sha256='+hashlib.sha256(value.encode()).hexdigest())
        return value
    finally:
        run([P,'box','down',name],required=False); p.stdin.close(); p.wait(timeout=90); err.close(); run([P,'disk','rm',name+'-home'])
def copied(name):
    path=W/'c/pinfold/profiles'/name/'Containerfile'
    if not path.exists(): run([P,'profile','new',name,'--from','default'])
    return path
def native(p,want_install_cache):
    headers={}; states={}; text=p.stdout+p.stderr
    for line in text.splitlines():
        m=re.match(r'(#\d+) \[(.*)\]',line)
        if m: headers[m[1]]=re.sub(r'^linux/\S+ ', '', m[2])
        m=re.match(r'(#\d+) (CACHED|DONE|ERROR)',line)
        if m: states[m[1]]=m[2]
    base=any(v.startswith('base ') and states.get(k)=='CACHED' for k,v in headers.items())
    installed=next((states.get(k) for k,v in headers.items() if v.startswith('agents ') and 'RUN set -eu;' in v),None)
    resolved=bool(re.search(r'#\d+\s+[0-9.]+\s+resolving stable agent releases',text))
    emit('native-progress resolve-executed='+str(resolved)+' base-cached='+str(base)+' installer-state='+str(installed))
    check(base and resolved and installed==('CACHED' if want_install_cache else 'DONE'),'native cached/executed stage proof')
    for line in text.splitlines():
        if re.match(r'#\d+ (?:CACHED|DONE|ERROR)',line) or re.search(r'#\d+\s+[0-9.]+\s+resolving stable agent releases',line): emit('native '+line)
if MODE=='observe': observe(sys.argv[2] if len(sys.argv)>2 else 'localhost/pinfold/profile-default:latest')
elif MODE=='pairdefault':
    ref='localhost/pinfold/profile-default:latest'; ids=[]; sizes=[]
    for i in range(2):
        build(['--profile','default']); ids.append(identity(ref)); sizes.append(backing())
    check(ids[0]==ids[1],'unchanged managed image identity'); emit('unchanged-pair stable-id=true builder-bytes='+json.dumps(sizes))
    check(LINUX or sizes[1]-sizes[0]<=64*1024**2,'cached Mac builder growth exceeds tens of MiB'); observe(ref)
    path=copied('probe-cache-native'); good=path.read_text(); ref='localhost/pinfold/profile-probe-cache-native:latest'; before=identity(ref)
    try:
        path.write_text(good+'\nRUN echo fixture-native-cache-stop >&2; exit 42\n'); p,_=build(['--profile','probe-cache-native'],required=False)
        check(p.returncode and identity(ref)==before and 'fixture-native-cache-stop' in p.stderr,'failure without publication'); native(p,True)
    finally: path.write_text(good)
elif MODE=='changed':
    tag=json.loads(run(['curl','-q','-fsSL','https://api.github.com/repos/earendil-works/pi/releases/tags/v1.5.1']).stdout)
    check(tag['tag_name']=='v1.5.1' and not tag['draft'] and not tag['prerelease'],'publisher confirms existing stable Pi 1.5.1')
    emit('publisher-confirmed existing stable release v1.5.1')
    path=copied('probe-cache-version'); original=path.read_text(); ref='localhost/pinfold/profile-probe-cache-version:latest'
    build(['--profile','probe-cache-version']); before=identity(ref)
    changed=original.replace('https://api.github.com/repos/earendil-works/pi/releases/latest','https://api.github.com/repos/earendil-works/pi/releases/tags/v1.5.1')
    check(changed!=original,'release probe replacement')
    try:
        path.write_text(changed+'\nRUN echo fixture-native-version-stop >&2; exit 42\n'); p,_=build(['--profile','probe-cache-version'],required=False)
        check(p.returncode and identity(ref)==before and 'fixture-native-version-stop' in p.stderr,'changed release failure preserves refs'); native(p,False)
        path.write_text(changed); build(['--profile','probe-cache-version']); value=observe(ref)
        check(value.splitlines()[0]=='1.5.1','selected published Pi release runs'); emit('changed-release installer-executed apt-cached corrected-publication=true')
    finally: path.write_text(original)
elif MODE=='nocache':
    ref='localhost/pinfold/profile-default:latest'; before=identity(ref); p,rows=build(['--profile','default','--no-cache']); observe(ref)
    check(before!=identity(ref),'no-cache rebuilt image'); check(not LINUX or any(r.get('host')=='deb.debian.org' for r in rows),'no-cache Debian steps executed')
    emit('managed --no-cache completed full rebuild; image-changed=true Debian-network='+str(any(r.get('host')=='deb.debian.org' for r in rows)))
elif MODE=='fullprojectpair':
    project=W/'project'; project.mkdir(exist_ok=True)
    if not (project/'.git').exists():
        run(['git','init','-q',project]); run(['git','-C',project,'-c','user.name=user','-c','user.email=user@fixture.invalid','commit','--allow-empty','-qm','fixture']); run([P,'config','edit'],project)
    report=json.loads(run([P,'config'],project).stdout); config=pathlib.Path(report['config'])
    config.write_text('profile="full"\nallow=[]\ncpus=2\nmemory="1G"\nmounts=[{host=".",kind="disk"}]\n')
    recipe=config.parent/'Containerfile'; good='FROM localhost/pinfold/profile-full:latest\nRUN printf project-ready > /project-fixture\n'; recipe.write_text(good); pairs=[]
    for i in range(2):
        build([],project); report=json.loads(run([P,'config'],project).stdout); pairs.append([identity('localhost/pinfold/profile-full:latest'),identity(report['image'])])
        result=run([P,'pi','--version'],project); check('image-outdated' not in result.stderr and result.returncode==0,'project launches after fresh profile build')
        for disk in report['project']['disks'].values(): run([P,'disk','rm',disk])
    check(pairs[0]==pairs[1],'full profile/project unchanged identities'); emit('full-derived-project pair-stable='+json.dumps(pairs)+' launch-no-image-outdated=true')
    try:
        recipe.write_text(good+'RUN install /required-agent-missing /usr/local/bin/fixture\n'); p,_=build([],project,False)
        check(p.returncode and 'required-agent-missing' in p.stderr and 'profile published' in p.stderr and identity(report['image'])==pairs[-1][1],'project failure preserves project after profile publication')
        recipe.write_text(good); build([],project); emit('project-failure refs-preserved=true profile-publication-reported=true corrected-build=true')
    finally: recipe.write_text(good)
elif MODE=='missing':
    path=copied('probe-cache-missing'); good=path.read_text(); ref='localhost/pinfold/profile-probe-cache-missing:latest'
    build(['--profile','probe-cache-missing']); before=identity(ref)
    bad=good.replace('    claude_base=https://downloads.claude.ai/claude-code-releases;', '    pi_asset=$(printf %s "$pi_asset" | jq -c \' .url += ".pinfold-missing"\'); \\\n    claude_base=https://downloads.claude.ai/claude-code-releases;')
    check(bad!=good,'missing download probe replacement')
    try:
        path.write_text(bad); p,_=build(['--profile','probe-cache-missing'],required=False)
        check(p.returncode and 'exit code: 22' in p.stderr and identity(ref)==before,'missing asset HTTP failure without publication')
        path.write_text(good); build(['--profile','probe-cache-missing']); emit('missing-download curl-HTTP-exit22 refs-preserved=true corrected-build=true')
    finally: path.write_text(good)
elif MODE=='denied':
    check(LINUX,'Apple build egress is unrestricted; denial case is Linux-only')
    path=W/'c/pinfold/profiles/probe-cache-denied/Containerfile'; path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text('FROM localhost/pinfold/profile-default:latest\nRUN curl -fsS https://example.com/ -o /fixture\n'); ref='localhost/pinfold/profile-probe-cache-denied:latest'; before=identity(ref)
    p,rows=build(['--profile','probe-cache-denied'],required=False)
    check(p.returncode and '403' in p.stderr and any(r.get('host')=='example.com' and r.get('reason')=='not allowlisted' for r in rows) and identity(ref)==before,'denied build host refuses without publication')
    build(['--profile','probe-cache-denied','--allow','example.com']); emit('denied-host HTTP403 reason=not allowlisted refs-preserved=true explicitly-admitted-correction=true')
else: raise RuntimeError('unknown mode')
emit('cache mode '+MODE+' passed; raw native output retained under '+str(W)+'; no box disks retained')
