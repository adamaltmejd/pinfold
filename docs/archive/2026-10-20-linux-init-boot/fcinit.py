#!/usr/bin/env python3
"""Throwaway host side of pinfold's Linux init protocol (#140), for the
labvm check. Not pinfold code: #141 writes the real client. No jail (#139).

  fcinit.py up STATE --firecracker F --kernel K --init INIT --root ROOT
            [--mem MIB] [--disk PATH:GUEST:rw|ro]... [--fill GUEST=DIR]...
            [--env NAME]...      # values come from this process's env
            [--defer-env 1]      # leave ENV to a later `env`
  fcinit.py env STATE [NAME]...
  fcinit.py exec STATE [--tty] [--resize R,C] [--workdir D] -- ARGV...
  fcinit.py stat STATE
  fcinit.py down STATE
"""
import json
import os
import socket
import stat as st
import struct
import subprocess
import sys
import threading

BOOT_PORT, EXEC_PORT, CONTROL_PORT = 1000, 1024, 1025
BOOT, READY, FAILED, ENV, ENV_SET, EXEC, STAT = b"BRFVvXS"
STDIN, STDIN_END, RESIZE, STDOUT, STDERR, EXIT = b"ieworx"
UNMOUNT, CLEAN = b"UC"
BOOT_ARGS = "console=ttyS0 reboot=k panic=1 pci=off init=/pinfold -- init boot"


def frame(kind, payload=b""):
    return bytes([kind]) + struct.pack(">I", len(payload)) + payload


def string(value):
    value = value if isinstance(value, bytes) else os.fsencode(value)
    return struct.pack(">I", len(value)) + value


def readn(sock, count):
    data = b""
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise EOFError("connection ended")
        data += chunk
    return data


def read_frame(sock):
    head = readn(sock, 5)
    return head[0], readn(sock, struct.unpack(">I", head[1:])[0])


def copy_stream(root):
    """The `init receive` stream of ROOT: dirs, files (exec bit kept),
    symlinks as links; other types skipped, as disk.rs does."""
    for top, dirs, files in os.walk(root):
        for name in sorted(dirs + files):
            path = os.path.join(top, name)
            rel = os.path.relpath(path, root)
            info = os.lstat(path)
            if st.S_ISLNK(info.st_mode):
                yield b"l" + string(rel) + string(os.readlink(path))
            elif st.S_ISDIR(info.st_mode):
                yield b"d" + string(rel)
            elif st.S_ISREG(info.st_mode):
                tag = b"x" if info.st_mode & 0o111 else b"f"
                with open(path, "rb") as file:
                    data = file.read()
                yield tag + string(rel) + struct.pack(">Q", len(data)) + data
    yield b"\0"


def guest(state, port):
    conn = socket.socket(socket.AF_UNIX)
    conn.connect(f"{state}/vsock/v.sock")
    conn.sendall(f"CONNECT {port}\n".encode())
    line = b""
    while not line.endswith(b"\n"):
        line += readn(conn, 1)
    if not line.startswith(b"OK"):
        raise RuntimeError(f"vsock connect {port}: {line!r}")
    return conn


def proxy_standin(path):
    """Stands in for pinfold's proxy on the box's 3128 socket."""
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(path)
    listener.listen(16)

    def serve():
        while True:
            conn, _ = listener.accept()
            conn.sendall(b"proxy-standin\n")
            conn.close()

    threading.Thread(target=serve, daemon=True).start()


def up(state, args):
    opts = {"--disk": [], "--fill": [], "--env": [], "--mem": ["512"]}
    it = iter(args)
    for key in it:
        opts.setdefault(key, []).append(next(it))
    os.makedirs(f"{state}/vsock", mode=0o700)
    drives = [
        {"drive_id": "init", "path_on_host": opts["--init"][0],
         "is_root_device": True, "is_read_only": True},
        {"drive_id": "root", "path_on_host": opts["--root"][0],
         "is_root_device": False, "is_read_only": True},
    ]
    attachments = []
    for index, spec in enumerate(opts["--disk"]):
        path, guest_path, mode = spec.rsplit(":", 2)
        drives.append({"drive_id": f"d{index}", "path_on_host": path,
                       "is_root_device": False, "is_read_only": mode != "rw"})
        attachments.append((f"vd{chr(ord('c') + index)}", guest_path, mode == "rw"))
    config = {
        "boot-source": {"kernel_image_path": opts["--kernel"][0], "boot_args": BOOT_ARGS},
        "drives": drives,
        "machine-config": {"vcpu_count": 2, "mem_size_mib": int(opts["--mem"][0])},
        "vsock": {"guest_cid": 3, "uds_path": f"{state}/vsock/v.sock"},
    }
    with open(f"{state}/config.json", "w") as file:
        json.dump(config, file, indent=1)
    proxy_standin(f"{state}/vsock/v.sock_3128")
    boot_listener = socket.socket(socket.AF_UNIX)
    boot_listener.bind(f"{state}/vsock/v.sock_{BOOT_PORT}")
    boot_listener.listen(1)
    with open(f"{state}/console.log", "wb") as console:
        vmm = subprocess.Popen(
            [opts["--firecracker"][0], "--no-api", "--config-file", f"{state}/config.json"],
            stdin=subprocess.DEVNULL, stdout=console, stderr=subprocess.STDOUT)
    with open(f"{state}/vmm.pid", "w") as file:
        file.write(str(vmm.pid))
    boot_listener.settimeout(30)
    conn, _ = boot_listener.accept()
    boot_listener.close()
    os.unlink(f"{state}/vsock/v.sock_{BOOT_PORT}")
    fills = [spec.split("=", 1) for spec in opts["--fill"]]
    payload = struct.pack(">II", os.getuid(), os.getgid()) + string("vdb")
    payload += struct.pack(">I", len(attachments))
    for device, guest_path, writable in attachments:
        payload += string(device) + string(guest_path) + bytes([writable])
    payload += struct.pack(">I", len(fills))
    for guest_path, _ in fills:
        payload += string(guest_path)
    conn.sendall(frame(BOOT, payload))
    for _, source in fills:
        for chunk in copy_stream(source):
            conn.sendall(chunk)
    kind, detail = read_frame(conn)
    if kind != READY:
        print(f"failed: {detail.decode(errors='replace')}", flush=True)
        vmm.kill()
        sys.exit(1)
    if not opts.get("--defer-env"):
        send_env(state, opts["--env"])
    print("ready", flush=True)
    sys.exit(vmm.wait())


def send_env(state, names):
    env = struct.pack(">I", len(names) + 2) + string("PATH") + string("/usr/bin:/bin:/usr/sbin:/sbin")
    env += string("HTTPS_PROXY") + string("http://127.0.0.1:3128")
    for name in names:
        env += string(name) + string(os.environ[name])
    conn = guest(state, EXEC_PORT)
    conn.sendall(frame(ENV, env))
    if read_frame(conn)[0] != ENV_SET:
        sys.exit("env not set")


def run_exec(state, args):
    tty, resize, workdir = False, None, b""
    while args[0] != "--":
        flag = args.pop(0)
        if flag == "--tty":
            tty = True
        elif flag == "--resize":
            resize = [int(n) for n in args.pop(0).split(",")]
        elif flag == "--workdir":
            workdir = os.fsencode(args.pop(0))
    argv = args[1:]
    conn = guest(state, EXEC_PORT)
    payload = struct.pack(">I", len(argv)) + b"".join(string(a) for a in argv)
    payload += string(workdir) + struct.pack(">BHH", tty, 24, 80)
    payload += struct.pack(">I", 1) + string("TERM") + string("xterm")
    conn.sendall(frame(EXEC, payload))
    if resize:
        conn.sendall(frame(RESIZE, struct.pack(">HH", *resize)))

    def forward():
        while chunk := os.read(0, 65536):
            conn.sendall(frame(STDIN, chunk))
        conn.sendall(frame(STDIN_END))

    threading.Thread(target=forward, daemon=True).start()
    while True:
        kind, data = read_frame(conn)
        if kind == STDOUT:
            sys.stdout.buffer.write(data)
            sys.stdout.flush()
        elif kind == STDERR:
            sys.stderr.buffer.write(data)
            sys.stderr.flush()
        elif kind == EXIT:
            os._exit(struct.unpack(">i", data)[0] & 0xFF)
        elif kind == FAILED:
            sys.exit(f"failed: {data.decode(errors='replace')}")


def run_stat(state):
    conn = guest(state, EXEC_PORT)
    conn.sendall(frame(STAT))
    kind, data = read_frame(conn)
    names = ["memory.current", "memory.peak", "pids.current", "pids.max", "oom_kills"]
    values = {}
    for index, name in enumerate(names):
        present, value = struct.unpack(">BQ", data[index * 9:index * 9 + 9])
        values[name] = value if present else None
    print(json.dumps(values))


def run_down(state):
    conn = guest(state, CONTROL_PORT)
    conn.sendall(frame(UNMOUNT))
    kind, detail = read_frame(conn)
    print("clean" if kind == CLEAN else f"failed: {detail.decode()}", flush=True)
    with open(f"{state}/vmm.pid") as file:
        os.kill(int(file.read()), 9)


def main():
    command, state, *rest = sys.argv[1:]
    if command == "up":
        up(state, rest)
    elif command == "exec":
        run_exec(state, rest)
    elif command == "env":
        send_env(state, rest)
    elif command == "stat":
        run_stat(state)
    elif command == "down":
        run_down(state)


main()
