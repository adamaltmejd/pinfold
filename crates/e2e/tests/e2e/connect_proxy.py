"""Hold the first real artifact CONNECT, then relay authenticated TLS unchanged."""
import selectors
import socket
import sys

selector = selectors.DefaultSelector()
listener = socket.socket()
listener.bind(("127.0.0.1", 0))
listener.listen(8)
selector.register(listener, selectors.EVENT_READ, "accept")
selector.register(sys.stdin, selectors.EVENT_READ, "control")
print(listener.getsockname()[1], flush=True)
held = None
first = True
connections = 0
peers = {}


def close_pair(stream):
    other = peers.pop(stream, None)
    for endpoint in (stream, other):
        if endpoint is not None:
            peers.pop(endpoint, None)
            selector.unregister(endpoint)
            endpoint.close()


def tunnel(client, target):
    upstream = socket.create_connection(target, timeout=15)
    client.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
    peers[client] = upstream
    peers[upstream] = client
    selector.register(client, selectors.EVENT_READ, "relay")
    selector.register(upstream, selectors.EVENT_READ, "relay")


while True:
    events = selector.select(timeout=60)
    if not events:
        raise TimeoutError("artifact fixture inactive for 60 seconds")
    for key, _ in events:
        stream, kind = key.fileobj, key.data
        if kind == "control":
            command = sys.stdin.readline()
            if not command:
                sys.exit(0)
            if command != "release\n" or held is None:
                raise ValueError("unexpected artifact fixture command")
            client, target = held
            selector.unregister(client)
            held = None
            tunnel(client, target)
        elif kind == "accept":
            client, _ = listener.accept()
            client.settimeout(15)
            connections += 1
            if connections > 8:
                raise ValueError("too many artifact connections")
            head = bytearray()
            while not head.endswith(b"\r\n\r\n"):
                byte = client.recv(1)
                if not byte or len(head) >= 16384:
                    raise ValueError("invalid CONNECT head")
                head.extend(byte)
            method, authority, _ = head.split(b"\r\n", 1)[0].decode().split()
            host, port = authority.rsplit(":", 1)
            if method != "CONNECT" or port != "443" or host not in {
                "github.com", "release-assets.githubusercontent.com",
                "objects.githubusercontent.com",
            }:
                raise ValueError("unexpected artifact target")
            target = (host, int(port))
            if first:
                first = False
                held = (client, target)
                selector.register(client, selectors.EVENT_READ, "held")
                print("held", flush=True)
            else:
                tunnel(client, target)
        elif kind == "held":
            if stream.recv(1):
                raise ValueError("client sent TLS before CONNECT succeeded")
            selector.unregister(stream)
            stream.close()
            held = None
            print("closed", flush=True)
        elif stream in peers:
            try:
                data = stream.recv(65536)
                if data:
                    peers[stream].sendall(data)
                else:
                    close_pair(stream)
            except (ConnectionError, TimeoutError):
                close_pair(stream)
