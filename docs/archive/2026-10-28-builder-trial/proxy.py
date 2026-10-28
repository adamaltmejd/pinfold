"""Trial allowlist proxy: no production framing/SNI/security equivalence."""
import json,select,socket,socketserver,threading,time,urllib.parse
from pathlib import Path
ALLOW={"registry-1.docker.io","auth.docker.io","production.cloudflare.docker.com","production.cloudfront.docker.com","docker-images-prod.6aa30f8b08e16409b46e0173d6de2f56.r2.cloudflarestorage.com","deb.debian.org","security.debian.org","github.com","release-assets.githubusercontent.com","registry.npmjs.org","cli.github.com","api.github.com"}
lock=threading.Lock()
class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(120)
        line=self.rfile.readline(16384)
        if not line:return
        method,target,version=line.decode("latin1").strip().split(" ")
        headers=[]
        while True:
            x=self.rfile.readline(16384)
            if x in (b"\r\n",b"\n",b""):break
            headers.append(x)
        if method=="CONNECT":
            host,port=target.rsplit(":",1); port=int(port); path=""
        else:
            url=urllib.parse.urlsplit(target); host=url.hostname; port=url.port or 80; path=url.path or "/"
        allowed=host in ALLOW and port in (80,443)
        event={"host":host,"port":port,"method":method,"path":path,"allowed":allowed}
        with lock:
            with open("proxy.jsonl","a") as f:f.write(json.dumps(event)+"\n")
        if not allowed:
            self.wfile.write(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 15\r\nConnection: close\r\n\r\nnot allowlisted")
            return
        try:
            upstream=socket.create_connection((host,port),timeout=30)
            if method=="CONNECT":self.wfile.write(b"HTTP/1.1 200 Connection established\r\n\r\n")
            else:
                upstream.sendall(f"{method} {path}{('?' + url.query) if url.query else ''} {version}\r\n".encode()+b"".join(x for x in headers if not x.lower().startswith(b"proxy-connection:"))+b"Connection: close\r\n\r\n")
            self.wfile.flush()
            with upstream:
                while True:
                    ready,_,_=select.select([self.connection,upstream],[],[],120)
                    if not ready:return
                    for source in ready:
                        data=source.recv(65536)
                        if not data:return
                        (upstream if source is self.connection else self.connection).sendall(data)
        except OSError as e:
            with lock:
                with open("proxy.jsonl","a") as f:f.write(json.dumps({"error":str(e),"host":host})+"\n")
class Server(socketserver.ThreadingUnixStreamServer):
    daemon_threads=True
    allow_reuse_address=True
def start(path):
    try:Path(path).unlink()
    except FileNotFoundError:pass
    server=Server(path,Handler)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    return server
