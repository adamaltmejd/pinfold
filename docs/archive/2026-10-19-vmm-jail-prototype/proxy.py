# Stand-in for pinfold's egress proxy (PR #111 ran the real one over vsock):
# HTTP CONNECT on a unix socket; api.github.com:443 allowed, all else 403.
import os, socket, sys, threading
path, logf = sys.argv[1], sys.argv[2]
def pipe(a, b):
    try:
        while (d := a.recv(65536)):
            b.sendall(d)
    except OSError:
        pass
    finally:
        for s in (a, b):
            try: s.shutdown(socket.SHUT_RDWR)
            except OSError: pass
def handle(c):
    req = b""
    while b"\r\n\r\n" not in req:
        d = c.recv(4096)
        if not d: return
        req += d
    line = req.split(b"\r\n")[0].decode()
    verb, target = line.split()[:2]
    ok = verb == "CONNECT" and target == "api.github.com:443"
    with open(logf, "a") as f: f.write(f"{'allow' if ok else 'deny'} {line}\n")
    if not ok:
        c.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"); c.close(); return
    u = socket.create_connection(("api.github.com", 443))
    c.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
    threading.Thread(target=pipe, args=(c, u), daemon=True).start()
    pipe(u, c)
s = socket.socket(socket.AF_UNIX)
s.bind(path); os.chmod(path, 0o600); s.listen(64)
while True:
    c, _ = s.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
