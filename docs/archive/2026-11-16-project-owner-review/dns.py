#!/usr/bin/env python3
"""Linux only: sudo unshare --mount --net --fork --kill-child python3 dns.py BIN IMAGE UID GID."""
import http.server, json, os, pathlib, pwd, socket, struct, subprocess as sp, sys, tempfile, threading, time
assert sys.platform == 'linux' and os.geteuid() == 0
assert os.readlink('/proc/self/ns/mnt') != os.readlink('/proc/1/ns/mnt'), 'requires private mount namespace'
assert os.readlink('/proc/self/ns/net') != os.readlink('/proc/1/ns/net'), 'requires private network namespace'
uid, gid = int(sys.argv[3]), int(sys.argv[4]); assert uid > 0 and gid > 0
home = pwd.getpwuid(uid).pw_dir
sp.run(['mount', '--make-rprivate', '/'], check=True)
sp.run(['ip', 'link', 'set', 'lo', 'up'], check=True)
namespace = pathlib.Path(tempfile.mkdtemp(prefix='pfr-dns-ns.', dir='/tmp'))
resolv = namespace/'resolv.conf'; resolv.write_text('nameserver 127.0.0.1\noptions timeout:10 attempts:5\n')
sp.run(['mount', '--bind', str(resolv), '/etc/resolv.conf'], check=True)
os.chown(namespace, uid, gid)
queries = []; cancel_seen = threading.Event(); deadline_seen = threading.Event(); dns_done = threading.Event()
dns_socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); dns_socket.bind(('127.0.0.1', 53))
def dns_serve():
    while not dns_done.is_set():
        packet, address = dns_socket.recvfrom(4096)
        if dns_done.is_set(): return
        if len(packet) < 16: continue
        offset = 12; labels = []
        while packet[offset]:
            size = packet[offset]; offset += 1
            labels.append(packet[offset:offset+size].decode('ascii')); offset += size
        offset += 1; kind, family = struct.unpack('!HH', packet[offset:offset+4]); end = offset+4
        host = '.'.join(labels).lower(); queries.append(host)
        if host == 'held-cancel.invalid': cancel_seen.set(); continue
        if host == 'api.github.com': deadline_seen.set(); continue
        if host != 'control.invalid': continue
        answer = b''
        if kind == 1 and family == 1:
            answer = b'\xc0\x0c'+struct.pack('!HHIH', 1, 1, 0, 4)+socket.inet_aton('127.0.0.1')
        response = packet[:2]+struct.pack('!HHHHH', 0x8180, 1, int(bool(answer)), 0, 0)+packet[12:end]+answer
        dns_socket.sendto(response, address)
dns_thread = threading.Thread(target=dns_serve); dns_thread.start()
# Binding the private DNS fixture is the only privileged operation the driver needs.
os.setgroups([]); os.setgid(gid); os.setuid(uid); os.environ['HOME'] = home
old = pathlib.Path(__file__).with_name('crossing.py')
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0], str(old), 'exec'), globals())
receipts = []
class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        receipts.append(self.path); self.send_response(200); self.end_headers(); self.wfile.write(b'DNS-CONTROL')
    def log_message(self, *args): pass
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
server_thread = threading.Thread(target=server.serve_forever); server_thread.start()
P.mkdir(); git(P, 'init', '-q', '-b', 'review'); git(P, '-c', 'user.name=user', '-c', 'user.email=user@pinfold.invalid', 'commit', '--allow-empty', '-qm', 'fixture')
run(['config', 'edit']); c = W/'c/pinfold/projects'/ident(P)/'pinfold.toml'
c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=["api.github.com"]\nroutes={{cancel="held-cancel.invalid:{server.server_port}",control="control.invalid:{server.server_port}"}}\n')
held = None
try:
    run(['start']); cancelled_owner = loc(P)
    assert run(['--', 'curl', '-fsS', 'http://control/control-before']).stdout == b'DNS-CONTROL'
    held = sp.Popen([B, '--', 'curl', '-sS', '-o', '/dev/null', '-w', '%{http_code}', 'http://cancel/held'],
                    cwd=P, env=E, stdin=sp.DEVNULL, stdout=sp.PIPE, stderr=sp.PIPE)
    assert cancel_seen.wait(90), 'real system resolver did not reach private DNS barrier'
    started = time.monotonic(); run(['stop']); elapsed = time.monotonic()-started
    output, errors = held.communicate(timeout=90)
    assert held.returncode == 125 and b'signal' in errors, 'incomplete client did not report owner stop'
    assert elapsed < 10 and not locator(P).exists(), 'teardown waited for blocked system resolver'
    terminal = json.loads((W/'s/pinfold/runs'/cancelled_owner['generation']/'terminal.json').read_text())
    assert terminal['reason'] == 'signal' and terminal['detail'] is None
    assert '/held' not in receipts
    emit(f'DNS cancel: real-query=held-cancel.invalid stop=0 incomplete-client=125/signal teardown-seconds={elapsed:.2f} late-HTTP-connections=0')
    run(['start']); current = loc(P)
    assert run(['--', 'curl', '-fsS', 'http://control/control-after']).stdout == b'DNS-CONTROL'
    started = time.monotonic(); refused = run(['--', 'curl', '-sS', '-o', '/dev/null', '-w', '%{http_code}', 'http://api.github.com/deadline']); elapsed = time.monotonic()-started
    assert deadline_seen.is_set() and refused.stdout == b'502' and 8 <= elapsed < 16
    decisions = [json.loads(line) for line in (W/'s/pinfold/runs'/current['generation']/'egress.jsonl').read_text().splitlines()]
    assert any(line.get('host') == 'api.github.com' and line.get('reason') == 'resolve failed' for line in decisions)
    assert run(['--', 'curl', '-fsS', 'http://control/control-restored']).stdout == b'DNS-CONTROL'; same(P, current)
    assert receipts == ['/control-before', '/control-after', '/control-restored']
    emit(f'DNS deadline: real-query=api.github.com HTTP=502/resolve-failed seconds={elapsed:.2f} allowed-fixture=200/DNS-CONTROL same-generation=True')
finally:
    if held is not None and held.poll() is None:
        held.terminate(); held.communicate(timeout=90)
    for args in (['stop'], ['disk', 'rm', 'pi-'+ident(P)+'-home'], ['disk', 'rm', 'pi-'+ident(P)+'-project']):
        result = invoke(args); cleanup_ok &= result.returncode == 0
    server.shutdown(); server.server_close(); server_thread.join()
    dns_done.set(); wake = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); wake.sendto(b'close', ('127.0.0.1', 53)); wake.close(); dns_thread.join(); dns_socket.close()
    emit(f'cleanup own-boxes/disks={cleanup_ok} private-resolver-mount=namespace-only fixture=$W'); assert cleanup_ok
