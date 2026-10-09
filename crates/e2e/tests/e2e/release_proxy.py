"""Host HTTPS release fixture, reached through curl's normal proxy settings."""

import hashlib
import json
import pathlib
import socketserver
import ssl
import sys
import threading
import time

root = pathlib.Path(sys.argv[1])
asset = sys.argv[2]
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(root / "cert.pem", root / "key.pem")
download_release = threading.Event()


class Proxy(socketserver.BaseRequestHandler):
    def handle(self):
        # Consume CONNECT without buffering TLS bytes from the next request.
        request = bytearray()
        while not request.endswith(b"\r\n\r\n"):
            byte = self.request.recv(1)
            if not byte:
                return
            request.extend(byte)
            if request == b"RELEASE\n":
                download_release.set()
                self.request.sendall(b"released\n")
                return
        self.request.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
        with context.wrap_socket(self.request, server_side=True) as stream:
            headers = stream.makefile("rb")
            path = headers.readline().decode().split()[1]
            while headers.readline() not in (b"\r\n", b""):
                pass
            if path == "/repos/adamaltmejd/pinfold/releases/latest":
                with (root / "checks").open("ab") as log:
                    log.write(b"check\n")
                if (root / "offline").exists():
                    start = time.monotonic()
                    stream.settimeout(4)
                    try:
                        closed = not stream.recv(1)
                    except (ConnectionResetError, ssl.SSLEOFError):
                        closed = True
                    except TimeoutError:
                        closed = False
                    print(json.dumps({
                        "event": "stalled-check-closed",
                        "closed": closed,
                        "seconds": time.monotonic() - start,
                    }), flush=True)
                    return
                body = json.dumps(
                    {
                        "tag_name": "v999.0.0",
                        "draft": False,
                        "prerelease": False,
                    }
                ).encode()
            elif path == "/adamaltmejd/pinfold/releases/download/v999.0.0/" + asset:
                if (root / "hold-download").exists():
                    print("download", flush=True)
                    download_release.wait()
                body = (root / "release").read_bytes()
            elif path == "/adamaltmejd/pinfold/releases/download/v999.0.0/SHA256SUMS":
                digest = hashlib.sha256((root / "release").read_bytes()).hexdigest()
                lines = [f"{digest}  {asset}"]
                if (root / "bad-checksum").exists():
                    # Wrong digest under the requested name catches a skipped
                    # comparison; the correct digest under another name
                    # catches a digest check that skips asset selection.
                    lines = [f"{'0' * 64}  {asset}", f"{digest}  {asset}.other"]
                body = ("\n".join(lines) + "\n").encode()
            else:
                stream.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                return
            stream.sendall(
                f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode()
                + body
            )


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True


with Server(("127.0.0.1", 0), Proxy) as server:
    print(server.server_address[1], flush=True)
    server.serve_forever()
