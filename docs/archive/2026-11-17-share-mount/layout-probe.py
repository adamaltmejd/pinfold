import os, pathlib, subprocess, tempfile, shutil
root = pathlib.Path(tempfile.mkdtemp(prefix='pf279.', dir='/tmp')).resolve()
name = 'probe-share279-layout'
def run(args):
    p = subprocess.run(args, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    print(p.stdout.strip(), flush=True)
    return p.returncode
try:
    share = root / 'share'
    (share / '.git').mkdir(parents=True)
    (share / '.git' / 'config').write_text('protected\n')
    print('host alternate-case resolves:', (share / '.GIT' / 'config').exists(), flush=True)
    run(['container', 'run', '--rm', '--name', name, '--network', 'none', '--read-only', '--cap-drop', 'ALL', '--user', f'{os.getuid()}:{os.getgid()}', '--mount', f'type=bind,source={share},target=/project', '--mount', f'type=bind,source={share}/.git,target=/project/.git,readonly', '--entrypoint', '/bin/sh', 'localhost/pinfold/profile-default', '-c', 'printf "direct: "; (printf direct > /project/.git/config) 2>&1; printf "alternate: "; (printf alternate > /project/.GIT/config) 2>&1; cat /project/.git/config; cat /project/.GIT/config'])
    print('host protected bytes:', (share / '.git' / 'config').read_text(), flush=True)
finally:
    # --rm removes this probe only; force cleanup if startup failed.
    subprocess.run(['container', 'delete', '--force', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    shutil.rmtree(root)
    print('fixture removed:', not root.exists(), flush=True)
