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
resolv = namespace/'resolv.conf'; resolv.write_text('nameserver 127.0.0.1\noptions timeout:60 attempts:5\n')
sp.run(['mount', '--bind', str(resolv), '/etc/resolv.conf'], check=True)
os.chown(namespace, uid, gid)
pending_dns = []; release_dns = threading.Event()
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
        if host == 'api.github.com':
            deadline_seen.set()
            if not release_dns.is_set(): pending_dns.append((packet,address,end)); continue
            response=packet[:2]+struct.pack('!HHHHH',0x8183,1,0,0,0)+packet[12:end]; dns_socket.sendto(response,address); continue
        if host != 'control.invalid': continue
        answer = b''
        if kind == 1 and family == 1:
            answer = b'\xc0\x0c'+struct.pack('!HHIH', 1, 1, 0, 4)+socket.inet_aton('127.0.0.1')
        response = packet[:2]+struct.pack('!HHHHH', 0x8180, 1, int(bool(answer)), 0, 0)+packet[12:end]+answer
        dns_socket.sendto(response, address)
dns_thread = threading.Thread(target=dns_serve); dns_thread.start()
# Binding the private DNS fixture is the only privileged operation the driver needs.
os.setgroups([]); os.setgid(gid); os.setuid(uid); os.environ['HOME'] = home
old = pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
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
try:
    run(['start']); current=loc(P)
    listed=[json.loads(x) for x in run(['box','list','--label','dev.pinfold.owner']).stdout.splitlines()]
    owner=next(x['owner'] for x in listed if x['name']==current['name'])
    def workers():
        return sum(p.joinpath('comm').read_text().strip()=='proxy-lookup' for p in pathlib.Path('/proc',str(owner),'task').iterdir())
    def batch(count):
        result=run(['--','sh','-c',f'for i in $(seq 1 {count}); do curl -sS -o /dev/null -w "%{{http_code}}\n" http://api.github.com/held & done; wait'])
        assert result.stdout.splitlines()==[b'502']*count,result.stdout
    assert run(['--','curl','-fsS','http://control/before']).stdout==b'DNS-CONTROL'
    batch(48); first=workers(); assert first==48,first
    batch(16); held=workers(); assert held==64,held
    before=len(queries); began=time.monotonic(); batch(16); elapsed=time.monotonic()-began
    assert elapsed<3 and workers()==64 and len(queries)==before,(elapsed,workers(),len(queries)-before)
    decisions=[json.loads(x) for x in (W/'s/pinfold/runs'/current['generation']/'egress.jsonl').read_text().splitlines()]
    assert sum(x.get('reason')=='resolve failed' for x in decisions)==80
    emit(f'resolver-cap: timed-out batches=48+16 workers-held=64 additional=16/502/resolve-failed seconds={elapsed:.2f} workers-after=64 additional-DNS=0')
    release_dns.set()
    for packet,address,end in pending_dns:
        response=packet[:2]+struct.pack('!HHHHH',0x8183,1,0,0,0)+packet[12:end]; dns_socket.sendto(response,address)
    deadline=time.monotonic()+10
    while workers():
        assert time.monotonic()<deadline,'resolver slots did not release after real DNS reply'
        threading.Event().wait(.01)
    assert run(['--','curl','-fsS','http://control/after']).stdout==b'DNS-CONTROL'; same(P,current)
    emit('resolver-cap control: real DNS replies released all64 workers; same-owner allowed DNS/HTTP=200/DNS-CONTROL')
finally:
    for args in (['stop'],['disk','rm','pi-'+ident(P)+'-home'],['disk','rm','pi-'+ident(P)+'-project']): cleanup_ok &= invoke(args).returncode==0
    server.shutdown(); server.server_close(); server_thread.join()
    dns_done.set(); wake=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);wake.sendto(b'close',('127.0.0.1',53));wake.close();dns_thread.join();dns_socket.close()
    emit(f'cleanup own-boxes/disks={cleanup_ok} namespace-only=True'); assert cleanup_ok
