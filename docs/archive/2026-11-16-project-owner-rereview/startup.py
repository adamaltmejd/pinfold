#!/usr/bin/env python3
"""Manual startup teardown: startup.py BIN IMAGE [stop|exited|audit-log|all]."""
import http.server, queue, signal, threading, time
import hashlib, json, os, pathlib, select, subprocess as sp, sys
old = pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0], str(old), 'exec'), globals())
ONLY = sys.argv[3] if len(sys.argv)>3 else 'all'
P.mkdir(); git(P, 'init', '-q', '-b', 'rereview'); (P/'tracked').write_text('FIRST\n'); git(P, 'add', 'tracked')
git(P, '-c', 'user.name=user', '-c', 'user.email=user@pinfold.invalid', 'commit', '-qm', 'fixture'); run(['config', 'edit'])
C = W/'c/pinfold/projects'/ident(P)/'pinfold.toml'
receipt, release, unexpected = threading.Event(), threading.Event(), threading.Event()
class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        seen, resumed = receipt, release
        if self.path == '/hold': seen.set(); assert resumed.wait(180), 'sync fixture was not released'
        elif self.path == '/trigger': unexpected.set()
        try: self.send_response(200); self.end_headers(); self.wfile.write(b'ROUTE-CONTROL')
        except (BrokenPipeError, ConnectionResetError): pass
    def log_message(self, *args): pass
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
server_thread = threading.Thread(target=server.serve_forever); server_thread.start()
C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\nroutes={{sync="127.0.0.1:{server.server_port}"}}\n')
def owner_info(observed):
    rows = [json.loads(row) for row in run(['box', 'list', '--label', 'dev.pinfold.owner']).stdout.splitlines()]
    owner = next(int(row['owner']) for row in rows if row['name']==observed['name'] and row['labels']['dev.pinfold.generation']==observed['generation'])
    claim = json.loads((H/(hashlib.sha256(('box:'+observed['name']).encode()).hexdigest()+'.owner')).read_text())
    return owner, claim

def mac_remover(owner, name, done, found, paused):
    try:
        deadline = time.monotonic()+90
        while not done.is_set():
            assert time.monotonic()<deadline, 'native teardown helper absent'
            rows = sp.run(['ps', '-axo', 'pid=,ppid='], capture_output=True, check=True).stdout.splitlines()
            for row in rows:
                pid, parent = map(int, row.split())
                if parent != owner: continue
                args = sp.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True).stdout
                if name.encode() in args and any(verb in args for verb in (b' kill ', b' stop ', b' rm ')):
                    os.kill(pid, signal.SIGSTOP); paused.append(pid)
                    assert b'T' in sp.run(['ps', '-p', str(pid), '-o', 'state='], capture_output=True, check=True).stdout
                    found.put(pid); return
    except BaseException as error: found.put(error)

def case(mode):
    global receipt, release, unexpected
    receipt, release, unexpected = threading.Event(), threading.Event(), threading.Event()
    run(['start'])
    assert run(['--', 'curl', '-fsS', 'http://sync/control']).stdout==b'ROUTE-CONTROL'
    hook = '#!/bin/sh\nsync\nexec curl -fsS http://sync/hold\n'
    run(['--', 'sh', '-c', 'printf %s "$1" >.git/hooks/post-merge; chmod +x .git/hooks/post-merge; sync', 'sh', hook]); run(['stop'])
    (P/'changed').write_text(mode+'\n'); git(P, 'add', 'changed'); git(P, '-c', 'user.name=user', '-c', 'user.email=user@pinfold.invalid', 'commit', '-qm', mode)
    starter = sp.Popen([B, '--', 'printf', 'MUST-NOT-RUN'], cwd=P, env=E, stdin=sp.DEVNULL, stdout=sp.PIPE, stderr=sp.PIPE); children.append(starter)
    stopper = trigger = watcher = None; done = threading.Event(); found = queue.Queue(); paused = []; owner = None
    try:
        assert receipt.wait(90), 'initial sync did not reach real HTTP barrier'
        observed = loc(P); generation = observed['generation']; assert generation
        owner, claim = owner_info(observed); began = time.monotonic()
        if mode == 'stop':
            if sys.platform == 'linux':
                vmm = int((pathlib.Path(claim['state_dir'])/'vmm').read_text().split()[0]); os.kill(vmm, signal.SIGSTOP)
                assert pathlib.Path(f'/proc/{vmm}/status').read_text().split('State:', 1)[1].lstrip().startswith('T')
            else:
                watcher = threading.Thread(target=mac_remover, args=(owner, observed['name'], done, found, paused)); watcher.start()
            stopper = sp.Popen([B, 'stop'], cwd=P, env=E, stdin=sp.DEVNULL, stdout=sp.PIPE, stderr=sp.PIPE)
            if sys.platform != 'linux':
                held = found.get(timeout=90); assert isinstance(held, int), held
            assert not select.select([starter.stdout], [], [], 12)[0] and starter.poll() is None, 'bootstrap returned during bounded teardown'
            if sys.platform != 'linux': os.kill(held, signal.SIGCONT); paused.clear()
            stop_out, stop_err = stopper.communicate(timeout=180); assert stopper.returncode==0, (stop_out, stop_err)
            expected = 'stopped'
        elif mode == 'exited':
            if sys.platform == 'linux':
                vmm = int((pathlib.Path(claim['state_dir'])/'vmm').read_text().split()[0]); os.kill(vmm, signal.SIGKILL)
            else: sp.run(['container', 'kill', '--signal', 'KILL', observed['name']], check=True, capture_output=True, timeout=30)
            expected = 'exited'
        else:
            log = W/'s/pinfold/runs'/generation/'egress.jsonl'; assert log.is_file()
            with log.open('r+b') as stream: stream.truncate(128*1024*1024)
            trigger = sp.Popen([B, 'box', 'exec', observed['name'], '--', 'curl', '-fsS', 'http://sync/trigger'], cwd=P, env=E, stdin=sp.DEVNULL, stdout=sp.PIPE, stderr=sp.PIPE)
            expected = 'audit-log'
        output, errors = starter.communicate(timeout=180); elapsed = time.monotonic()-began
        assert starter.returncode==125 and not output and expected.encode() in errors and generation.encode() in errors and b'pinfold box inspect' in errors and b'owner-timeout' not in errors, (starter.returncode, output, errors)
        if mode=='stop': assert elapsed>10
        terminal = json.loads((W/'s/pinfold/runs'/generation/'terminal.json').read_text())
        assert terminal['reason']==('signal' if mode=='stop' else expected) and not locator(P).exists(), terminal
        if trigger is not None: trigger.communicate(timeout=90)
        emit(f'startup-{mode}: real HTTP-held sync; client=125/{expected} full-generation=True inspect=True owner-timeout=False CMD=absent teardown-seconds={elapsed:.2f} terminal={terminal["reason"]}')
        release.set()
        control = run(['--', 'sh', '-c', 'rm -f .git/hooks/post-merge; printf STARTUP-CONTROL; sync'])
        assert control.stdout==b'STARTUP-CONTROL'; run(['stop']); assert not locator(P).exists()
        emit(f'startup-{mode} control: checked restart=0/STARTUP-CONTROL fixture hook removed normal teardown=True')
    finally:
        release.set(); done.set()
        if watcher is not None: watcher.join(timeout=10); assert not watcher.is_alive()
        for pid in paused:
            parent = sp.run(['ps', '-p', str(pid), '-o', 'ppid='], capture_output=True).stdout.strip()
            if parent==str(owner).encode(): os.kill(pid, signal.SIGCONT)
        for child in (starter, stopper, trigger):
            if child is not None and child.poll() is None: child.terminate(); child.communicate(timeout=180)
        run(['stop'])
try:
    for mode in ('stop', 'exited', 'audit-log'):
        if ONLY in ('all', mode): case(mode)
finally:
    release.set()
    for args in (['stop'], ['disk', 'rm', 'pi-'+ident(P)+'-home'], ['disk', 'rm', 'pi-'+ident(P)+'-project']): cleanup_ok &= invoke(args).returncode==0
    server.shutdown(); server.server_close(); server_thread.join()
    emit(f'cleanup own-boxes/disks={cleanup_ok} fixture=$W'); assert cleanup_ok
