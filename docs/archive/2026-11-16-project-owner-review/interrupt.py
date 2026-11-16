#!/usr/bin/env python3
"""Real public crossing interrupted by owner stop: interrupt.py BIN IMAGE."""
import http.server, json, pathlib, subprocess as sp, sys, threading
old = pathlib.Path(__file__).with_name('waits.py')
exec(compile(old.read_text().split('def line(')[0], str(old), 'exec'), globals())
receipt, release = threading.Event(), threading.Event()
class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        assert self.path == '/transfer'; receipt.set(); assert release.wait(150), 'fixture release absent'
        try: self.send_response(200); self.end_headers(); self.wfile.write(b'TRANSFER-RELEASED')
        except BrokenPipeError: pass
    def log_message(self, *args): pass
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
thread = threading.Thread(target=server.serve_forever); thread.start(); crossing = None
try:
    C.write_text(C.read_text()+f'routes={{hold="127.0.0.1:{server.server_port}"}}\n'); run(['start']); observed = loc(P)
    hook = '#!/bin/sh\nif [ "$1" = prepared ]; then exec curl -fsS http://hold/transfer; fi\n'
    run(['--', 'sh', '-c', 'mkdir -p .git/hooks; printf %s "$1" >.git/hooks/reference-transaction; chmod +x .git/hooks/reference-transaction', 'sh', hook])
    (P/'changed').write_text('CHANGED\n'); git(P, 'add', 'changed')
    git(P, '-c', 'user.name=user', '-c', 'user.email=user@pinfold.invalid', 'commit', '-qm', 'changed')
    crossing = sp.Popen([B, 'push'], cwd=P, env=E, stdin=sp.DEVNULL, stdout=sp.PIPE, stderr=sp.PIPE)
    assert receipt.wait(90), 'real guest reference transaction did not reach HTTP barrier'
    same(P, observed); run(['stop']); output, errors = crossing.communicate(timeout=90)
    assert crossing.returncode == 125, ('crossing shutdown status', crossing.returncode, output, errors[:500])
    assert b'signal' in errors and observed['generation'].encode() in errors
    assert ('pinfold box inspect '+observed['generation']).encode() in errors, 'inspection command absent'
    assert not any(json.loads(line).get('event') in ('pushed', 'fetched') for line in output.splitlines())
    emit('crossing interrupted: real prepared-ref hook HTTP receipt; stop=0 push=125 reason=signal full-generation=True inspect-command=True claimed-success=False')
    release.set(); run(['start']); current = loc(P)
    # SIGKILL of the fixture's prepared transaction can leave its own Git lock.
    assert run(['--', 'sh', '-c', 'rm -f .git/hooks/reference-transaction .git/refs/remotes/host/review.lock .git/packed-refs.lock; printf RESTART-CONTROL']).stdout == b'RESTART-CONTROL'
    result = json.loads(run(['push']).stdout)
    assert result['event'] == 'pushed' and result['box'] == current['name']
    actual = run(['--', 'git', 'rev-parse', 'refs/remotes/host/review']).stdout.strip()
    assert actual == git(P, 'rev-parse', 'HEAD'); same(P, current); run(['stop'])
    emit('crossing corrected control: fixture hook/abandoned transaction lock removed; restart marker=RESTART-CONTROL; public push=0/pushed; guest-ref=independent-host-HEAD')
finally:
    release.set()
    if crossing is not None and crossing.poll() is None: crossing.terminate(); crossing.communicate(timeout=90)
    for args in (['stop'], ['disk', 'rm', 'pi-'+ident(P)+'-home'], ['disk', 'rm', 'pi-'+ident(P)+'-project']): cleanup_ok &= invoke(args).returncode == 0
    server.shutdown(); server.server_close(); thread.join()
    emit(f'cleanup own-boxes/disks={cleanup_ok} fixture=$W'); assert cleanup_ok
