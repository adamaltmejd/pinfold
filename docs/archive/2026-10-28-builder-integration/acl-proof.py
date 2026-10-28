#!/usr/bin/env python3
"""Manual converter component proof using the compiled production guest init.

Run on an exclusively leased Linux/KVM host as its non-root user. Requires
mkfs.ext4, setfacl, getfacl, setcap, getcap, and passwordless sudo for the two
owned fixture files. No package installation or network request runs here.

This bypasses the host CLI, VMM jail, common claims and production receiver.
It proves only this fixture's converter/agent guest path. The host never
extracts an OCI layer or reads/mounts a guest-writable filesystem.
"""

import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import select
import shutil
import socket
import stat
import struct
import subprocess
import tarfile
import time
import tomllib

REPO = Path(__file__).resolve().parents[3]
ROOT_SHA = "710189b13d814e4026e1095bd393dad99465e4042ced5dae069c59fd79cbaa80"
ROOT_BYTES = 1073741824
LIMIT = 512 * 1024**2
JSON_LIMIT = 1024**2
HEAD_LIMIT = 8192
IMAGE = "application/vnd.oci.image.manifest.v1+json"
CONFIG = "application/vnd.oci.image.config.v1+json"
LAYER = "application/vnd.oci.image.layer.v1.tar"


def require(condition, detail):
    if not condition:
        raise ValueError(detail)


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write(path, data, mode=0o600):
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode), "wb") as out:
        out.write(data)


def json_write(path, value):
    write(path, (json.dumps(value, indent=2, sort_keys=True) + "\n").encode())


def tool(name):
    found = shutil.which(name)
    if found:
        return found
    for directory in ("/usr/sbin", "/sbin"):
        path = Path(directory) / name
        if path.is_file() and os.access(path, os.X_OK):
            return str(path)
    raise FileNotFoundError(name)


def run(*argv):
    return subprocess.run(
        [str(arg) for arg in argv], check=True, capture_output=True
    ).stdout


def digest_hex(value):
    require(
        value.startswith("sha256:")
        and len(value) == 71
        and all(char in "0123456789abcdef" for char in value[7:]),
        "invalid SHA-256 descriptor",
    )
    return value[7:]


def object_path(layout, descriptor):
    require(not descriptor.get("urls"), "external OCI descriptor URL")
    path = layout / "blobs/sha256" / digest_hex(descriptor["digest"])
    metadata = path.lstat()
    require(stat.S_ISREG(metadata.st_mode), "OCI object is not a regular file")
    require(metadata.st_size == descriptor["size"] <= LIMIT, "OCI object size")
    require(sha(path) == descriptor["digest"][7:], "OCI object digest")
    return path


def add_blob(layout, data, media):
    digest = hashlib.sha256(data).hexdigest()
    target = layout / "blobs/sha256" / digest
    if not target.exists():
        write(target, data)
    return {"mediaType": media, "digest": f"sha256:{digest}", "size": len(data)}


def copy_base(source, target, work):
    require((source / "index.json").stat().st_size <= JSON_LIMIT, "base index size")
    index_bytes = (source / "index.json").read_bytes()
    index = json.loads(index_bytes)
    require(len(index["manifests"]) == 1, "base must be the validated runnable layout")
    descriptor = index["manifests"][0]
    require(descriptor["mediaType"] == IMAGE, "base manifest media type")
    manifest_file = object_path(source, descriptor)
    require(manifest_file.stat().st_size <= JSON_LIMIT, "base manifest size")
    manifest = json.loads(manifest_file.read_bytes())
    require(manifest["config"]["mediaType"] == CONFIG, "base config media type")
    config_file = object_path(source, manifest["config"])
    require(config_file.stat().st_size <= JSON_LIMIT, "base config size")
    config = json.loads(config_file.read_bytes())
    for name, data in [
        ("index", index_bytes),
        ("manifest", manifest_file.read_bytes()),
        ("config", config_file.read_bytes()),
    ]:
        write(work / f"source-base-{name}.json", data)
    require(
        config["os"] == "linux" and config["architecture"] == "amd64",
        "base platform",
    )
    require(
        config["rootfs"]["type"] == "layers"
        and len(config["rootfs"]["diff_ids"]) == len(manifest["layers"]),
        "base rootfs graph",
    )
    total = 0
    for item in [descriptor, manifest["config"], *manifest["layers"]]:
        path = object_path(source, item)
        total += path.stat().st_size
        require(total <= LIMIT, "base object payload limit")
        destination = target / "blobs/sha256" / item["digest"][7:]
        if not destination.exists():
            shutil.copyfile(path, destination)
            destination.chmod(0o600)
    return manifest, config


def fixture(work, uid, gid):
    owned = work / "host-fixture"
    owned.mkdir(mode=0o755)
    owned.chmod(0o755)
    file = owned / "owned"
    write(file, b"owned-fixture\n", 0o751)
    os.chmod(file, 0o751)
    run(tool("setfacl"), "-m", f"u:345:r--,u:{uid}:r--", file)
    os.setxattr(file, "user.phase6b", b"yes")
    os.link(file, owned / "hard")
    run(tool("sudo"), "-n", tool("chown"), "123:456", file)
    os.symlink("/fixtures/owned", owned / "sym")
    write(owned / "gone", b"removed-in-layer-two\n", 0o644)
    cap = owned / "cap"
    shutil.copyfile("/bin/true", cap)
    cap.chmod(0o755)
    run(tool("sudo"), "-n", tool("chown"), "0:0", cap)
    run(tool("sudo"), "-n", tool("setcap"), "cap_net_bind_service=ep", cap)
    run(tool("setfacl"), "-m", f"d:u:345:r-x,d:u:{uid}:r-x", owned)
    access = os.getxattr(file, "system.posix_acl_access")
    default = os.getxattr(owned, "system.posix_acl_default")
    capability = os.getxattr(cap, "security.capability")
    require(access and default and capability, "host kernel did not supply fixture metadata")
    observed = file.stat()
    require(
        (observed.st_uid, observed.st_gid, stat.S_IMODE(observed.st_mode)) == (123, 456, 0o751),
        "fixture ownership/mode changed",
    )
    require(observed.st_ino == (owned / "hard").stat().st_ino, "fixture hardlink")
    expected = {
        "host_uid": uid,
        "host_gid": gid,
        "owner": [observed.st_uid, observed.st_gid, stat.S_IMODE(observed.st_mode)],
        "getfacl": run(tool("getfacl"), "-cpn", file).decode(),
        "access_acl": base64.b64encode(access).decode(),
        "default_acl": base64.b64encode(default).decode(),
        "capability": base64.b64encode(capability).decode(),
        "user": "yes",
        "hardlink": True,
        "symlink": os.readlink(owned / "sym"),
        "gone_before_whiteout": (owned / "gone").is_file(),
    }
    json_write(work / "expected.json", expected)
    return owned, expected


def fixture_layers(layout, owned):
    first = io.BytesIO()
    with tarfile.open(fileobj=first, mode="w", format=tarfile.PAX_FORMAT,
                      encoding="utf-8", errors="surrogateescape") as archive:
        for name in ["", "owned", "hard", "sym", "cap", "gone"]:
            path = owned / name
            info = archive.gettarinfo(str(path), arcname=f"fixtures/{name}".rstrip("/"))
            info.uname = info.gname = ""
            info.mtime = 0
            attributes = {}
            if name == "":
                attributes["system.posix_acl_default"] = os.getxattr(path, "system.posix_acl_default")
            elif name == "owned":
                for key in ["system.posix_acl_access", "user.phase6b"]:
                    attributes[key] = os.getxattr(path, key)
            elif name == "cap":
                attributes["security.capability"] = os.getxattr(path, "security.capability")
            if attributes:
                info.pax_headers = {"hdrcharset": "BINARY"}
                info.pax_headers.update({
                    f"SCHILY.xattr.{key}": value.decode("utf-8", "surrogateescape")
                    for key, value in attributes.items()
                })
            if info.isfile():
                with path.open("rb") as source:
                    archive.addfile(info, source)
            else:
                archive.addfile(info)
    second = io.BytesIO()
    with tarfile.open(fileobj=second, mode="w", format=tarfile.PAX_FORMAT) as archive:
        whiteout = tarfile.TarInfo("fixtures/.wh.gone")
        whiteout.mode = 0o000
        whiteout.size = whiteout.mtime = 0
        archive.addfile(whiteout)
    raw = [first.getvalue(), second.getvalue()]
    return [add_blob(layout, data, LAYER) for data in raw]


def prepare_input(base, work, owned):
    input_tree = work / "input-tree"
    for path in ["context", "recipe", "bases/0/blobs/sha256"]:
        (input_tree / path).mkdir(parents=True, exist_ok=True)
    layout = input_tree / "bases/0"
    manifest, config = copy_base(base, layout, work)
    layers = fixture_layers(layout, owned)
    config["rootfs"]["diff_ids"].extend(layer["digest"] for layer in layers)
    config.setdefault("config", {}).setdefault("Labels", {})["dev.example.fixture"] = "acl-proof"
    config.setdefault("history", []).extend(
        {"created_by": f"independent owned ACL fixture layer {number}"} for number in [1, 2]
    )
    config_descriptor = add_blob(layout, json.dumps(config, sort_keys=True).encode(), CONFIG)
    manifest["config"] = config_descriptor
    manifest["layers"].extend(layers)
    manifest.pop("subject", None)
    manifest.pop("artifactType", None)
    descriptor = add_blob(layout, json.dumps(manifest, sort_keys=True).encode(), IMAGE)
    descriptor["platform"] = {"os": "linux", "architecture": "amd64"}
    json_write(layout / "index.json", {
        "schemaVersion": 2, "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [descriptor],
    })
    json_write(layout / "oci-layout", {"imageLayoutVersion": "1.0.0"})
    # Base config/manifest bytes were verified before modifying the in-memory
    # copies. Keep only objects referenced by the independently constructed graph.
    referenced = {config_descriptor["digest"], *(layer["digest"] for layer in manifest["layers"]), descriptor["digest"]}
    for path in (layout / "blobs/sha256").iterdir():
        if f"sha256:{path.name}" not in referenced:
            path.unlink()
    write(input_tree / "recipe/Dockerfile", b"FROM --platform=linux/amd64 prepared\n", 0o644)
    json_write(input_tree / "job.json", {
        "phase": "convert", "cache": False, "labels": {},
        "bases": [{"reference": "prepared", "digest": descriptor["digest"], "directory": "/input/bases/0"}],
    })
    json_write(input_tree / "source-policy.json", {
        "version": 1, "rules": [{"action": "DENY", "selector": {
            "identifier": "docker-image://*", "match_type": "WILDCARD",
        }}],
    })
    return input_tree, descriptor


def disk(path, size, source=None):
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as out:
        out.truncate(size)
    argv = [tool("mkfs.ext4"), "-q", "-F", "-m", "0", "-O", "^has_journal"]
    if source:
        argv.extend(["-d", source])
    run(*argv, path)


def field(data):
    return struct.pack(">I", len(data)) + data


def frame(connection, kind, data):
    connection.sendall(kind + struct.pack(">I", len(data)) + data)


def exact(connection, size, deadline):
    data = bytearray()
    while len(data) < size:
        timeout(connection, deadline)
        chunk = connection.recv(size - len(data))
        if not chunk:
            raise EOFError("guest stream ended before its frame")
        data.extend(chunk)
    return bytes(data)


def timeout(connection, deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("proof guest deadline")
    connection.settimeout(remaining)


def read_frame(connection, deadline, limit=JSON_LIMIT):
    header = exact(connection, 5, deadline)
    length = struct.unpack(">I", header[1:])[0]
    require(length <= limit, "control/output header limit")
    return header[:1], exact(connection, length, deadline)


class VM:
    def __init__(self, work, role, executable, kernel, init, root, extra):
        self.dir = work / role
        self.dir.mkdir()
        self.socket = self.dir / "v.sock"
        endpoint = f"{self.socket}_1000"
        require(len(os.fsencode(endpoint)) < 108, "choose a shorter proof work directory")
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.child = None
        self.pidfd = None
        self.log = None
        self.deadline = time.monotonic() + 300
        drives = [
            {"drive_id": "init", "path_on_host": str(init), "is_root_device": True, "is_read_only": True},
            {"drive_id": "root", "path_on_host": str(root), "is_root_device": False, "is_read_only": True},
        ]
        drives.extend({
            "drive_id": f"disk{index}", "path_on_host": str(path),
            "is_root_device": False, "is_read_only": not writable,
        } for index, (path, writable) in enumerate(extra))
        config = {
            "boot-source": {
                "kernel_image_path": str(kernel),
                "boot_args": f"console=ttyS0 reboot=k panic=1 pci=off init=/pinfold -- init {role}",
            },
            "drives": drives,
            "machine-config": {"vcpu_count": 2 if role == "builder" else 1, "mem_size_mib": 2560 if role == "builder" else 1024},
            "vsock": {"guest_cid": 3, "uds_path": str(self.socket)},
        }
        json_write(self.dir / "config.json", config)
        self.listener.bind(endpoint)
        self.listener.listen(1)
        self.executable = executable

    def __enter__(self):
        self.log = (self.dir / "console.log").open("xb")
        try:
            self.child = subprocess.Popen(
                [self.executable, "--no-api", "--config-file", self.dir / "config.json"],
                stdin=subprocess.DEVNULL, stdout=self.log, stderr=subprocess.STDOUT,
            )
            self.pidfd = os.pidfd_open(self.child.pid)
            ready, _, _ = select.select([self.listener, self.pidfd], [], [], max(0, self.deadline - time.monotonic()))
            require(self.listener in ready, "VMM exited or timed out before its boot connection")
            connection, _ = self.listener.accept()
            self.listener.close()
            return self, connection
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def connect(self, port):
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            timeout(connection, self.deadline)
            connection.connect(str(self.socket))
            connection.sendall(f"CONNECT {port}\n".encode())
            line = bytearray()
            while len(line) < 64:
                byte = exact(connection, 1, self.deadline)
                line.extend(byte)
                if byte == b"\n":
                    break
            require(line.startswith(b"OK "), f"guest did not accept production port {port}")
            return connection
        except BaseException:
            connection.close()
            raise

    def __exit__(self, *_):
        if self.child is not None:
            if self.child.poll() is None:
                self.child.kill()
            self.child.wait()
            self.child = None
        if self.pidfd is not None:
            os.close(self.pidfd)
            self.pidfd = None
        self.listener.close()
        if self.log is not None:
            self.log.close()


def boot_request(uid, gid, extra):
    request = struct.pack(">IIB", uid, gid, 0) + field(b"vdb")
    request += struct.pack(">I", len(extra))
    for device, guest, writable in extra:
        request += field(device) + field(guest) + bytes([writable])
    return request + struct.pack(">I", 0)


def boot(vm, connection, uid, gid, extra):
    frame(connection, b"B", boot_request(uid, gid, extra))
    kind, data = read_frame(connection, vm.deadline)
    if kind == b"F":
        raise RuntimeError(data.decode(errors="replace"))
    require(kind == b"R" and not data, "production guest did not report ready")


def receive_root(connection, path, deadline):
    kind, data = read_frame(connection, deadline, HEAD_LIMIT)
    header = json.loads(data)
    require(kind == b"D" and set(header) == {"digest", "size"}, "converter root header")
    digest_hex(header["digest"])
    require(type(header["size"]) is int and 0 < header["size"] <= LIMIT, "converter root size")
    hashed = hashlib.sha256()
    remaining = header["size"]
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as out:
        while remaining:
            chunk = exact(connection, min(65536, remaining), deadline)
            out.write(chunk)
            hashed.update(chunk)
            remaining -= len(chunk)
        out.flush()
        os.fsync(out.fileno())
    require(f"sha256:{hashed.hexdigest()}" == header["digest"], "converter root digest")
    kind, data = read_frame(connection, deadline, HEAD_LIMIT)
    require(kind == b"E" and json.loads(data) == {}, "converter final success")
    timeout(connection, deadline)
    require(not connection.recv(1), "converter stream has trailing bytes")
    return header


def exec_guest(vm, argv):
    with vm.connect(1024) as connection:
        payload = struct.pack(">I", len(argv)) + b"".join(field(arg.encode()) for arg in argv)
        payload += field(b"") + struct.pack(">BHHI", 0, 0, 0, 0)
        frame(connection, b"X", payload)
        frame(connection, b"e", b"")
        outputs = {b"o": bytearray(), b"r": bytearray()}
        while True:
            kind, data = read_frame(connection, vm.deadline)
            if kind in outputs:
                outputs[kind].extend(data)
                require(sum(map(len, outputs.values())) <= JSON_LIMIT, "agent output limit")
            elif kind == b"x":
                require(len(data) == 4, "agent exit frame")
                return struct.unpack(">i", data)[0], bytes(outputs[b"o"]), bytes(outputs[b"r"])
            elif kind == b"F":
                raise RuntimeError(data.decode(errors="replace"))
            else:
                raise ValueError("unexpected agent exec frame")


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--builder-root", type=Path, required=True)
    parser.add_argument("--firecracker-archive", type=Path, required=True)
    parser.add_argument("--kernel", type=Path, required=True)
    parser.add_argument("--base-layout", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    for name in ["binary", "builder_root", "firecracker_archive", "kernel", "base_layout", "work"]:
        setattr(args, name, getattr(args, name).resolve())
    return args


def proof(args):
    require(os.getuid() != 0, "run as the disposable host's unprivileged observer")
    require(os.getuid() not in (123, 345), "observer UID conflicts with fixture identities")
    require(not args.work.is_relative_to(args.base_layout), "work directory is inside the base layout")
    require(args.builder_root.stat().st_size == ROOT_BYTES and sha(args.builder_root) == ROOT_SHA, "bootstrap root pin")
    pins = tomllib.loads((REPO / "crates/pinfold/vmm.toml").read_text())
    assets = {asset["as"]: asset for asset in pins["asset"] if asset["os-arch"] == "linux-x64"}
    require(sha(args.firecracker_archive) == assets["firecracker"]["sha256"], "Firecracker archive pin")
    require(sha(args.kernel) == assets["vmlinux"]["sha256"], "guest kernel pin")
    work = args.work
    write(work / "vmm.toml", (REPO / "crates/pinfold/vmm.toml").read_bytes())
    version = pins["version"].split("+")[0]
    executable = work / "firecracker"
    with tarfile.open(args.firecracker_archive) as archive:
        entries = [member for member in archive if Path(member.name).name == f"firecracker-v{version}-x86_64"]
        require(len(entries) == 1 and entries[0].isfile(), "pinned VMM named executable")
        write(executable, archive.extractfile(entries[0]).read(), 0o755)
    owned, expected = fixture(work, os.getuid(), os.getgid())
    source, descriptor = prepare_input(args.base_layout, work, owned)
    init_tree = work / "init-tree"
    init_tree.mkdir()
    for path in ["dev", "mnt"]:
        (init_tree / path).mkdir()
    shutil.copyfile(args.binary, init_tree / "pinfold")
    (init_tree / "pinfold").chmod(0o755)
    init_disk = work / "init.ext4"
    disk(init_disk, 128 * 1024**2, init_tree)
    input_disk = work / "input.ext4"
    input_bytes = sum(path.stat().st_size for path in source.rglob("*") if path.is_file())
    disk(input_disk, max(64 * 1024**2, input_bytes * 12 // 10 + 32 * 1024**2), source)
    scratch = work / "converter-work.ext4"
    disk(scratch, 3 * 1024**3)
    root = work / "converted.ext4"
    init_sha = sha(init_disk)
    input_sha = sha(input_disk)
    json_write(work / "inputs.json", {
        "binary": {"path": str(args.binary), "sha256": sha(args.binary)},
        "bootstrap": {"path": str(args.builder_root), "bytes": ROOT_BYTES, "sha256": ROOT_SHA},
        "init_disk_sha256": init_sha, "input_disk_sha256": input_sha,
        "fixture_manifest": descriptor,
    })
    with VM(work, "builder", executable, args.kernel, init_disk, args.builder_root,
            [(scratch, True), (input_disk, False)]) as (vm, connection):
        with connection:
            boot(vm, connection, 0, 0, [(b"vdc", b"/work", 1), (b"vdd", b"/input", 0)])
            received = receive_root(connection, root, vm.deadline)
    require(sha(args.builder_root) == ROOT_SHA, "bootstrap changed after converter")
    checks = []
    with VM(work, "boot", executable, args.kernel, init_disk, root, []) as (vm, connection):
        with connection:
            boot(vm, connection, os.getuid(), os.getgid(), [])
        with vm.connect(1024) as connection:
            environment = [(b"PATH", b"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")]
            payload = struct.pack(">I", len(environment)) + b"".join(field(k) + field(v) for k, v in environment)
            frame(connection, b"V", payload)
            require(read_frame(connection, vm.deadline) == (b"v", b""), "agent environment acknowledgement")
        cases = [
            (["id", "-u"], f"{os.getuid()}\n".encode()),
            (["stat", "-c", "%u:%g:%a", "/fixtures/owned"], b"123:456:751\n"),
            (["getfacl", "-cpn", "/fixtures/owned"], expected["getfacl"].encode()),
            (["getfattr", "--only-values", "-n", "system.posix_acl_access", "/fixtures/owned"], base64.b64decode(expected["access_acl"])),
            (["getfattr", "--only-values", "-n", "system.posix_acl_default", "/fixtures"], base64.b64decode(expected["default_acl"])),
            (["getfattr", "--only-values", "-n", "user.phase6b", "/fixtures/owned"], b"yes"),
            (["getfattr", "--only-values", "-n", "security.capability", "/fixtures/cap"], base64.b64decode(expected["capability"])),
            (["readlink", "/fixtures/sym"], b"/fixtures/owned\n"),
            (["test", "!", "-e", "/fixtures/gone"], b""),
        ]
        for number, (argv, wanted) in enumerate(cases):
            code, stdout, stderr = exec_guest(vm, argv)
            write(work / f"check-{number}.stdout", stdout)
            write(work / f"check-{number}.stderr", stderr)
            checks.append({"argv": argv, "code": code, "matches": stdout == wanted})
            check_file = work / f"check-{number}.json"
            json_write(check_file, checks[-1])
            require(code == 0 and stdout == wanted, f"agent fixture check {number} failed; see raw check files")
        code, stdout, stderr = exec_guest(vm, ["stat", "-c", "%d:%i", "/fixtures/owned", "/fixtures/hard"])
        write(work / "hardlink.stdout", stdout)
        write(work / "hardlink.stderr", stderr)
        lines = stdout.splitlines()
        require(code == 0 and len(lines) == 2 and lines[0] == lines[1], "agent hardlink relation")
        code, stdout, stderr = exec_guest(vm, ["getcap", "-n", "/fixtures/cap"])
        write(work / "getcap.stdout", stdout)
        write(work / "getcap.stderr", stderr)
        require(code == 0 and b"cap_net_bind_service=ep" in stdout, "agent capability metadata")
        with vm.connect(1025) as connection:
            frame(connection, b"U", b"")
            require(read_frame(connection, vm.deadline) == (b"C", b""), "agent clean shutdown acknowledgement")
    require(sha(root) == received["digest"][7:], "converted root changed during read-only agent boot")
    require(sha(args.builder_root) == ROOT_SHA, "bootstrap changed")
    require(sha(init_disk) == init_sha and sha(input_disk) == input_sha, "read-only inputs changed")
    json_write(work / "receipt.json", {
        "component_runtime_only": True,
        "normal_cli_consumer": False, "production_jail": False,
        "common_claims": False, "production_receiver": False,
        "binary": {"bytes": args.binary.stat().st_size, "sha256": sha(args.binary)},
        "inputs": {name: str(getattr(args, name)) for name in ["binary", "builder_root", "kernel", "base_layout", "firecracker_archive"]},
        "bootstrap": {"bytes": ROOT_BYTES, "sha256": ROOT_SHA},
        "vmm_pin": assets["firecracker"], "kernel_pin": assets["vmlinux"],
        "fixture_manifest": descriptor, "converted_root": received,
        "checks": checks, "hardlink": True, "security_capability": True,
        "host_uid": os.getuid(), "host_gid": os.getgid(),
    })
    # This disk was derivable guest work and is never read by the host.
    scratch.unlink()
    print(json.dumps({"success": True, "receipt": str(work / "receipt.json")}))


def main():
    args = arguments()
    require(not args.work.is_relative_to(args.base_layout), "work directory is inside the base layout")
    # Exclusive creation prevents failure reporting from touching an existing run.
    args.work.mkdir(mode=0o700)
    try:
        proof(args)
    except BaseException as error:
        json_write(args.work / "failure.json", {
            "component_runtime_only": True, "success": False,
            "error": f"{type(error).__name__}: {error}",
        })
        raise


if __name__ == "__main__":
    main()
