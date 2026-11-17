import hashlib, itertools, os, pathlib, shutil, subprocess, tempfile
root = pathlib.Path(tempfile.mkdtemp(prefix='pf279.', dir='/tmp')).resolve()
share = root / 'share'; other = root / 'other'; name = 'probe-share279-ruling'
(share / '.git').mkdir(parents=True); other.mkdir()
(share / '.git' / 'protected').write_bytes(b'protected\n')
for path in ['.git', 'project/.git', 'project']:
    (other / path).mkdir(parents=True, exist_ok=True)
def snapshot():
    return hashlib.sha256((share / '.git' / 'protected').read_bytes()).hexdigest()
before = snapshot()
try:
    for case, alias in [('root', '/var/run/project'), ('git', '/var/run/project/.git'), ('parent', '/var/run')]:
        mounts = {'share': f'type=bind,source={share},target=/run/project', 'caller': f'type=bind,source={other},target={alias}', 'git': f'type=bind,source={share}/.git,target=/run/project/.git,readonly'}
        for order in [('caller','share','git'), ('share','caller','git')]:
            args = ['container','run','--rm','--name',name,'--network','none','--read-only','--cap-drop','ALL','--user',f'{os.getuid()}:{os.getgid()}']
            for key in order: args += ['--mount', mounts[key]]
            command = 'for p in /run/project/.git/protected /var/run/project/.git/protected /var/run/project/.git/new /run/project/.git/new; do if printf changed > "$p"; then printf "WRITE %s\\n" "$p"; else printf "REFUSED %s\\n" "$p"; fi; done'
            args += ['--entrypoint','/bin/sh','localhost/pinfold/profile-default','-c',command]
            p = subprocess.run(args,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            print(f'case={case} order={",".join(order)} exit={p.returncode}', flush=True)
            print('\n'.join(x for x in p.stdout.splitlines() if x.startswith(('WRITE ', 'REFUSED '))),flush=True)
            unchanged = snapshot() == before and not (share / '.git' / 'new').exists()
            print(f'protected-host-bytes-unchanged={unchanged}',flush=True)
            if not unchanged: raise SystemExit('STOP: protected host byte changed')
            if p.returncode: raise SystemExit(p.stdout)
finally:
    subprocess.run(['container','delete','--force',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    shutil.rmtree(root)
    print(f'fixture-removed={not root.exists()}',flush=True)
