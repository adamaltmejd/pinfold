#!/usr/bin/env python3
"""Guarantee 36: real CONNECT traffic across the production idle deadline."""

import ctypes
import json
import os
from pathlib import Path
import queue
import select
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time


HOST = "api.github.com"
ADDRESS = "8.8.8.8"
IDLE_SECONDS = 300
TRAFFIC_SECONDS = 330
MARKER_INTERVAL = 30
MARKERS = TRAFFIC_SECONDS // MARKER_INTERVAL + 1
SCHEDULING_TOLERANCE = 20
# Long Linux socket waits are rounded into coarse timer buckets. Stagger
# initially idle tunnels so a short rearmed wait cannot hide that delay.
IDLE_PHASES = 4
IDLE_PHASE_INTERVAL = 8
TUNNELS = ("upload", "download", *(f"idle-{n}" for n in range(IDLE_PHASES)))


def emit(event, **fields):
    print(json.dumps({"event": event, **fields}), flush=True)


def receive(stream, length):
    data = bytearray()
    while len(data) < length:
        chunk = stream.recv(length - len(data))
        if not chunk:
            raise RuntimeError(f"peer closed before {length} bytes arrived")
        data.extend(chunk)
    return bytes(data)


def markers(stream, stopped):
    began = time.monotonic()
    for sequence in range(MARKERS):
        # Schedule real traffic, rather than using a delay for readiness.
        deadline = began + sequence * MARKER_INTERVAL
        if stopped.wait(max(0, deadline - time.monotonic())):
            raise RuntimeError("traffic cancelled")
        stream.sendall(struct.pack("!Q", sequence))


def observe_markers(stream, report):
    last = None
    for expected in range(MARKERS):
        sequence = struct.unpack("!Q", receive(stream, 8))[0]
        if sequence != expected:
            raise RuntimeError(f"marker {sequence}, expected fixture marker {expected}")
        last = time.monotonic()
        report("marker", sequence=sequence, seconds=last)
    return last


def guest(direction):
    if direction not in TUNNELS:
        raise RuntimeError("unknown fixture direction")
    emit("waiting", direction=direction)
    connect_at = float(sys.stdin.readline())
    # This is the timed input to the idle scenario, not a readiness delay.
    threading.Event().wait(max(0, connect_at - time.monotonic()))
    raw = socket.create_connection(("127.0.0.1", 3128), timeout=15)
    raw.sendall(f"CONNECT {HOST}:443 HTTP/1.1\r\nHost: {HOST}:443\r\n\r\n".encode())
    head = bytearray()
    while not head.endswith(b"\r\n\r\n") and len(head) < 8192:
        head.extend(receive(raw, 1))
    if head.split(b" ", 2)[1:2] != [b"200"]:
        raise RuntimeError(f"CONNECT was refused: {head!r}")
    context = ssl.create_default_context(cafile="/fixture/ca.pem")
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.maximum_version = ssl.TLSVersion.TLSv1_3
    with context.wrap_socket(raw, server_hostname=HOST) as stream:
        stream.settimeout(IDLE_SECONDS + SCHEDULING_TOLERANCE + 10)
        token = {"upload": b"U", "download": b"D"}.get(direction)
        if token is None:
            token = b"I" + bytes([int(direction.removeprefix("idle-"))])
        stream.sendall(token)
        if receive(stream, 1) != b"R":
            raise RuntimeError("fixture did not acknowledge the TLS handshake")
        emit("ready", direction=direction, seconds=time.monotonic())
        if sys.stdin.readline() != "start\n":
            raise RuntimeError("controller did not start traffic")
        if direction == "upload":
            markers(stream, threading.Event())
        elif direction == "download":
            observe_markers(stream, emit)
        if stream.recv(1):
            raise RuntimeError("fixture sent unexpected data after its last marker")
        emit("closed", direction=direction, seconds=time.monotonic())
        # Keep the guest and its socket alive until the outside observer
        # has checked the closure; guest exit cannot be its cause.
        if sys.stdin.readline() != "release\n":
            raise RuntimeError("controller did not release the guest holder")


def host(binary, root, image):
    root = Path(root)
    events = queue.Queue()
    start = threading.Event()
    stopped = threading.Event()
    processes = {}
    audit = None
    log = None
    final = {}
    closed = {}
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("PINFOLD_")
    }
    env.update(
        {
            f"XDG_{kind}_HOME": str(root / kind.lower())
            for kind in ("STATE", "CACHE", "CONFIG")
        }
    )
    name = f"connect-proof-{os.getpid()}"

    def report(source, event, **fields):
        events.put((source, {"event": event, **fields}))

    def watch(source, process):
        try:
            for line in process.stdout:
                report(source, **json.loads(line))
        except Exception as error:
            report(source, "error", detail=str(error))
        report(source, "exit", code=process.wait())

    def launch(source, arguments):
        process = subprocess.Popen(
            arguments,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=sys.stderr,
            text=True,
            env=env,
        )
        processes[source] = process
        worker = threading.Thread(target=watch, args=(source, process), daemon=True)
        worker.start()
        return process

    def next_event(deadline):
        left = deadline - time.monotonic()
        if left <= 0:
            raise RuntimeError("fixture observation deadline exceeded")
        try:
            source, item = events.get(timeout=left)
        except queue.Empty as error:
            raise RuntimeError("fixture observation deadline exceeded") from error
        if item["event"] in ("error", "exit", "refused", "failed", "down"):
            raise RuntimeError(f"{source}: {item}")
        emit(
            item["event"],
            source=source,
            **{key: value for key, value in item.items() if key != "event"},
        )
        return source, item

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.maximum_version = ssl.TLSVersion.TLSv1_3
    # No delayed session tickets in the otherwise inactive direction.
    context.num_tickets = 0
    context.load_cert_chain(root / "server.pem", root / "server.key")
    listener = socket.socket()
    listener.bind((ADDRESS, 443))
    listener.listen(len(TUNNELS))
    listener.settimeout(45)

    def fixture(raw):
        direction = "unidentified"
        try:
            with context.wrap_socket(raw, server_side=True) as stream:
                stream.settimeout(IDLE_SECONDS + SCHEDULING_TOLERANCE + 10)
                token = receive(stream, 1)
                direction = {b"U": "upload", b"D": "download"}.get(token)
                if token == b"I":
                    direction = f"idle-{receive(stream, 1)[0]}"
                if direction not in TUNNELS:
                    raise RuntimeError("client did not name a fixture direction")
                stream.sendall(b"R")
                report(f"fixture-{direction}", "ready")
                if not start.wait(45):
                    raise RuntimeError("controller did not start the fixture")
                if direction == "upload":
                    observe_markers(
                        stream,
                        lambda event, **fields: report(
                            f"fixture-{direction}", event, **fields
                        ),
                    )
                elif direction == "download":
                    markers(stream, stopped)
                if stream.recv(1):
                    raise RuntimeError(
                        "client sent unexpected data after its last marker"
                    )
                report(f"fixture-{direction}", "closed", seconds=time.monotonic())
        except Exception as error:
            report(f"fixture-{direction}", "error", detail=str(error))

    def accept():
        try:
            for _ in TUNNELS:
                raw, _ = listener.accept()
                raw.settimeout(15)
                worker = threading.Thread(target=fixture, args=(raw,), daemon=True)
                worker.start()
        except Exception as error:
            report("fixture", "error", detail=str(error))

    acceptor = threading.Thread(target=accept, daemon=True)
    spec = {
        "name": name,
        "image": image,
        "mounts": [
            {"host": str(root / "guest"), "guest": "/fixture", "readonly": True}
        ],
        "egress": {"allow": [HOST]},
        "cpus": 1,
        "memory": "256M",
    }
    passed = False
    try:
        owner = launch("owner", [binary, "box", "up"])
        owner.stdin.write(json.dumps(spec) + "\n")
        owner.stdin.flush()
        source, ready = next_event(time.monotonic() + 60)
        if source != "owner" or ready["event"] != "ready":
            raise RuntimeError("owner did not report box readiness")
        log = Path(ready["egress_log"])
        # Peer EOF can precede the proxy thread's final log append. Wait on
        # the kernel's file event rather than sleeping or retrying the test.
        libc = ctypes.CDLL(None, use_errno=True)
        libc.inotify_init1.argtypes = [ctypes.c_int]
        libc.inotify_init1.restype = ctypes.c_int
        libc.inotify_add_watch.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint32,
        ]
        libc.inotify_add_watch.restype = ctypes.c_int
        audit = libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
        if audit < 0 or libc.inotify_add_watch(audit, os.fsencode(log), 2) < 0:
            raise OSError(ctypes.get_errno(), "watch the egress log")
        for direction in TUNNELS:
            launch(
                direction,
                [
                    binary,
                    "box",
                    "exec",
                    name,
                    "--",
                    "python3",
                    "/fixture/connect_tunnels_share_activity.py",
                    "guest",
                    direction,
                ],
            )
        waiting_guests = set(TUNNELS)
        deadline = time.monotonic() + 45
        while waiting_guests:
            source, item = next_event(deadline)
            if source not in waiting_guests or item["event"] != "waiting":
                raise RuntimeError(f"unexpected pre-connect event: {source}: {item}")
            waiting_guests.remove(source)
        acceptor.start()
        phases_begin = time.monotonic()
        for direction in TUNNELS:
            connect_at = phases_begin
            if direction.startswith("idle-"):
                connect_at += int(direction.removeprefix("idle-")) * IDLE_PHASE_INTERVAL
            processes[direction].stdin.write(f"{connect_at}\n")
            processes[direction].stdin.flush()
        waiting = {*TUNNELS, *(f"fixture-{name}" for name in TUNNELS)}
        deadline = time.monotonic() + 45
        while waiting:
            source, item = next_event(deadline)
            if item["event"] != "ready" or source not in waiting:
                raise RuntimeError(f"unexpected fixture readiness: {source}: {item}")
            waiting.remove(source)
            if source.startswith("idle-"):
                final[source] = item["seconds"]
        began = time.monotonic()
        start.set()
        for direction in TUNNELS:
            processes[direction].stdin.write("start\n")
            processes[direction].stdin.flush()
        observer = {"fixture-upload": "upload", "download": "download"}
        deadline = began + TRAFFIC_SECONDS + IDLE_SECONDS + SCHEDULING_TOLERANCE + 10
        while len(closed) < 2 * len(TUNNELS):
            source, item = next_event(deadline)
            if item["event"] == "marker" and source in observer:
                if item["sequence"] == MARKERS - 1:
                    elapsed = item["seconds"] - began
                    if (
                        not IDLE_SECONDS
                        < elapsed
                        < TRAFFIC_SECONDS + SCHEDULING_TOLERANCE
                    ):
                        raise RuntimeError(
                            f"final {source} marker missed the production deadline: {elapsed}"
                        )
                    final[observer[source]] = item["seconds"]
            elif item["event"] == "closed":
                direction = source.removeprefix("fixture-")
                if direction not in final:
                    raise RuntimeError(f"{source} closed before the final marker")
                elapsed = item["seconds"] - final[direction]
                if (
                    not IDLE_SECONDS - 2
                    <= elapsed
                    <= IDLE_SECONDS + SCHEDULING_TOLERANCE
                ):
                    raise RuntimeError(
                        f"{source} idle closure outside production deadline: {elapsed}"
                    )
                if source in closed:
                    raise RuntimeError(f"duplicate closure from {source}")
                closed[source] = elapsed
            else:
                raise RuntimeError(f"unexpected traffic event: {source}: {item}")
        for direction in TUNNELS:
            if processes[direction].poll() is not None:
                raise RuntimeError(
                    f"{direction} guest exited before outside closure observation"
                )
        # The proxy's decision log identifies idle enforcement rather than
        # upstream EOF, a blocked write, or a transport failure.
        audit_deadline = time.monotonic() + 5
        while True:
            entries = [json.loads(line) for line in log.read_text().splitlines()]
            reasons = [
                entry["reason"]
                for entry in entries
                if entry.get("host") == HOST and entry.get("decision") == "closed"
            ]
            if len(reasons) >= len(TUNNELS):
                break
            left = audit_deadline - time.monotonic()
            if left <= 0 or not select.select([audit], [], [], left)[0]:
                raise RuntimeError("CONNECT closure was not recorded in the egress log")
            os.read(audit, 65536)
        if reasons != ["idle timeout"] * len(TUNNELS):
            raise RuntimeError(
                f"CONNECT closure reasons differ from the spec: {reasons}"
            )
        for direction in TUNNELS:
            processes[direction].stdin.write("release\n")
            processes[direction].stdin.flush()
            if processes[direction].wait(timeout=15) != 0:
                raise RuntimeError(f"{direction} guest holder failed")
        passed = True
    finally:
        if not passed:
            try:
                emit(
                    "diagnostic",
                    final_markers=final,
                    closed=closed,
                    processes={
                        source: process.poll() for source, process in processes.items()
                    },
                )
                if log is not None:
                    for line in log.read_text().splitlines():
                        emit("audit", entry=json.loads(line))
            except Exception:
                # Diagnostics must not replace the failure or skip teardown.
                pass
        stopped.set()
        listener.close()
        if "owner" in processes:
            try:
                processes["owner"].stdin.close()
            except BrokenPipeError:
                pass
        # Box teardown cancels every guest before reaping its runtime client.
        for source in ("owner", *TUNNELS):
            if source not in processes:
                continue
            process = processes[source]
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            if passed and process.returncode != 0:
                raise RuntimeError(
                    f"{source} failed during teardown: {process.returncode}"
                )
        subprocess.run([binary, "box", "prune"], env=env, check=True, timeout=30)
        if audit is not None and audit >= 0:
            os.close(audit)
    emit(
        "passed",
        guarantee=36,
        name="connect_tunnels_share_activity",
        idle_seconds=closed,
    )


if __name__ == "__main__":
    # Sabotage: make CONNECT reads expire independently, omit successful
    # writes from the shared clock, never expire that clock, or rely only
    # on coarse socket read timeouts for idle expiry. The final
    # directional marker or the subsequent spec-timed idle closure fails.
    if sys.argv[1:2] == ["guest"]:
        guest(sys.argv[2])
    elif sys.argv[1:2] == ["host"] and len(sys.argv) == 5:
        host(*sys.argv[2:])
    else:
        raise SystemExit(
            "usage: connect_tunnels_share_activity.py host BINARY ROOT IMAGE | guest DIRECTION"
        )
