import json, os, pathlib, shutil, subprocess, tempfile
B = pathlib.Path.home() / 'pinfold/target/x86_64-unknown-linux-musl/debug/pinfold'
W = pathlib.Path(tempfile.mkdtemp(prefix='pf279.', dir='/tmp'))
E = dict(os.environ, PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
P = W / 'source'; P.mkdir(); (P/'fixture').write_text('readonly-fixture\n')
def run(args, env=E, **kw):
    p = subprocess.run([str(a) for a in args], env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=600, **kw)
    return p
try:
    e = dict(E, XDG_CONFIG_HOME=str(W/'config'), XDG_STATE_HOME=str(W/'state'), XDG_CACHE_HOME=str(W/'cache'), PATH='/nonexistent')
    roots = [W/'config', W/'state', W/'cache', pathlib.Path(f'/tmp/pinfold-{os.getuid()}')]
    before = [p.exists() for p in roots]; print('refusal roots-before=',before, 'maintenance-due=True runtime-PATH=/nonexistent', flush=True)
    s = dict(name='probe-share279-linux-refuse', image='unavailable', mounts=[dict(host=str(P),guest='/project',kind='share')], env={'X':{'from':'UNSET_FIXTURE_ENV'}}, cpus=2,memory='1G')
    p = run([B,'box','up'],env=e,input=json.dumps(s)); print('share refusal rc=',p.returncode,p.stdout.strip(),flush=True)
    after = [p.exists() for p in roots]; print('refusal roots-after=',after,flush=True)
    assert before[:3] == after[:3] == [False]*3 and before == after and p.returncode==1
    v=json.loads(p.stdout); assert v['reason']=='spec' and 'share' in v['detail'] and 'Linux' in v['detail']
    E.update(XDG_CONFIG_HOME=str(W/'support-config'), XDG_STATE_HOME=str(W/'support-state'))
    stamp=W/'support-state/pinfold/maintenance';stamp.parent.mkdir(parents=True);stamp.touch()
    context=W/'context';context.mkdir();recipe=W/'Containerfile';recipe.write_text('FROM debian:trixie-slim\n')
    image=os.environ.get('PF279_LINUX_IMAGE')
    if not image:
        p=run([B,'image','build','probe-share279-linux','--containerfile',recipe,'--context',context]);print('image build rc=',p.returncode,p.stdout[-2500:],flush=True);assert p.returncode==0
        image=json.loads(p.stdout.splitlines()[-1])['ref']
    for n in [1,2]:
        s=dict(name=f'probe-share279-linux-{n}',image=image,mounts=[dict(host=str(P),guest='/readonly')],disks=[dict(name='probe-share279-linux-disk',guest='/work',size='1G')],env={'HOME':'/tmp'},cpus=2,memory='1G')
        owner=subprocess.Popen([str(B),'box','up'],env=E,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        try:
            owner.stdin.write(json.dumps(s)+'\n');owner.stdin.flush();line=owner.stdout.readline();print('supported box:',line.strip(),flush=True);assert json.loads(line)['event']=='ready'
            command='cat /readonly/fixture; if printf x > /readonly/fixture; then exit 1; fi; '
            command+= 'printf disk-fixture > /work/fixture; cat /work/fixture' if n==1 else 'cat /work/fixture; test "$(cat /work/fixture)" = disk-fixture'
            p=run([B,'box','exec',s['name'],'--','/bin/sh','-c',command]);print('supported exec rc=',p.returncode,p.stdout.strip(),flush=True);assert p.returncode==0
        finally:
            p=run([B,'box','down',s['name']]);print('supported down rc=',p.returncode,flush=True);owner.stdin.close();owner.wait(timeout=90);print('owner rc=',owner.returncode,owner.stdout.read().strip(),owner.stderr.read().strip(),flush=True)
    p=run([B,'disk','rm','probe-share279-linux-disk','--json']);print('disk cleanup rc=',p.returncode,p.stdout.strip(),flush=True);assert p.returncode==0
    p=run([B,'box','list','--label','dev.pinfold.owner']);print('box inventory:',p.stdout.strip() or 'empty',flush=True);assert p.returncode==0 and not p.stdout.strip()
finally:
    shutil.rmtree(W);print('linux fixture removed=',not W.exists(),flush=True)
