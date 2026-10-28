#!/usr/bin/env python3
"""Manual nested-KVM evidence for the bootstrap, outside pinfold's VMM jail."""

import base64
import gzip
import hashlib
import importlib.util
import json
import os
import select
import subprocess
import sys
import tarfile
import time
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
spec = importlib.util.spec_from_file_location(
    "bootstrap", REPO / "builder-root/build.py"
)
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def disk(path, size, source=None):
    with path.open("wb") as output:
        output.truncate(size)
    args = ["/usr/sbin/mkfs.ext4", "-q", "-F"]
    if source:
        args += ["-d", source]
    run(*args, path)


def boot(work, firecracker, role, data, input_disk, output_disk=None):
    drives = [
        {
            "drive_id": "root",
            "path_on_host": str(work / "root.ext4"),
            "is_root_device": True,
            "is_read_only": True,
        },
        {
            "drive_id": "work",
            "path_on_host": str(data),
            "is_root_device": False,
            "is_read_only": False,
        },
        {
            "drive_id": "input",
            "path_on_host": str(input_disk),
            "is_root_device": False,
            "is_read_only": True,
        },
    ]
    if output_disk:
        drives.append(
            {
                "drive_id": "output",
                "path_on_host": str(output_disk),
                "is_root_device": False,
                "is_read_only": False,
            }
        )
    config = {
        "boot-source": {
            "kernel_image_path": str(work / "vmlinux"),
            "boot_args": "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda ro init=/bin/bash",
        },
        "drives": drives,
        "machine-config": {"vcpu_count": 2, "mem_size_mib": 2560},
    }
    config_path = work / f"{role}-vm.json"
    config_path.write_text(json.dumps(config, indent=2) + "\n")
    vm = subprocess.Popen(
        [str(firecracker), "--no-api", "--config-file", str(config_path)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    output = bytearray()
    started = time.monotonic()

    def receive(marker):
        deadline = started + 300
        while marker not in output:
            if b"\r\nPROOF_FAILED\r\n" in output:
                raise RuntimeError(f"{role}: guest proof failed")
            if len(output) > 16 * 1024 * 1024:
                raise ValueError("serial proof output limit")
            if not select.select(
                [vm.stdout], [], [], max(0, deadline - time.monotonic())
            )[0]:
                raise TimeoutError(f"{role}: {marker!r}")
            chunk = os.read(vm.stdout.fileno(), 65536)
            if not chunk:
                raise RuntimeError(f"{role}: VM exited")
            output.extend(chunk)

    try:
        receive(b"root@(none):/# ")
        vm.stdin.write(b"stty -echo; printf '\\nSERIAL_READY\\n'\n")
        vm.stdin.flush()
        receive(b"\r\nSERIAL_READY\r\n")
        script = (HERE / "guest-common.sh").read_bytes() + (
            HERE / f"guest-{role}.sh"
        ).read_bytes()
        vm.stdin.write(
            b"mount -t tmpfs tmpfs /tmp; mount -t proc proc /proc; echo 0 >/proc/sys/user/max_user_namespaces; cat >/tmp/proof <<'PROOF'\n"
            + script
            + b"\nPROOF\nsetpriv --no-new-privs --bounding-set=-all,+chown,+dac_override,+fowner,+fsetid,+kill,+setgid,+setuid,+setpcap,+net_bind_service,+net_raw,+sys_chroot,+mknod,+audit_write,+setfcap,+sys_admin /bin/bash /tmp/proof || printf '\\nPROOF_FAILED\\n'\n"
        )
        vm.stdin.flush()
        receive(f"\r\n{role.upper()}_SUCCESS\r\n".encode())
        if role == "converter":
            encoded = (
                bytes(output)
                .split(b"\r\nCONVERTED_BEGIN\r\n", 1)[1]
                .split(b"CONVERTED_END", 1)[0]
            )
            compressed = work / "converted.ext4.gz"
            compressed.write_bytes(
                base64.b64decode(b"".join(encoded.splitlines()), validate=True)
            )
            with gzip.open(compressed, "rb") as source:
                raw_sha = hashlib.file_digest(source, "sha256").hexdigest()
            (work / "converted.sha256").write_text(raw_sha + "\n")
        return {
            "role": role,
            "seconds": round(time.monotonic() - started, 3),
            "success": True,
        }
    finally:
        vm.terminate()
        try:
            vm.wait(timeout=10)
        except subprocess.TimeoutExpired:
            vm.kill()
            vm.wait()
        output.extend(vm.stdout.read())
        (work / f"{role}-console.log").write_bytes(output)


def main():
    work = Path(sys.argv[2]).resolve()
    work.mkdir()
    dist = Path(sys.argv[1]).resolve()
    manifest = json.loads((dist / "manifest.json").read_text())
    asset = dist / manifest["artifact"]["name"]
    assert bootstrap.digest(asset) == manifest["artifact"]["sha256"]
    with tarfile.open(asset) as source:
        assert source.getnames() == ["root.ext4"]
        source.extractall(work, filter="data")
    assert bootstrap.digest(work / "root.ext4") == manifest["root"]["sha256"]
    vmm = tomllib.loads((REPO / "crates/pinfold/vmm.toml").read_text())
    for entry in vmm["asset"]:
        bootstrap.fetch(entry["url"], work / entry["as"], entry["sha256"])
    firecracker = work / "firecracker-bin"
    with tarfile.open(work / "firecracker") as source:
        members = [
            item
            for item in source
            if Path(item.name).name == "firecracker-v1.17.0-x86_64"
        ]
        assert len(members) == 1 and members[0].isfile()
        firecracker.write_bytes(source.extractfile(members[0]).read())
    firecracker.chmod(0o755)
    fixture = work / "fixture"
    fixture.mkdir()
    # busybox-static is only a proof fixture. It is not installed in the root.
    (fixture / "probe").write_bytes(Path("/bin/busybox").read_bytes())
    (fixture / "probe").chmod(0o755)
    (fixture / "Dockerfile").write_text(
        'FROM scratch\nCOPY probe /bin/sh\nCOPY fixture /fixture\nRUN ["/bin/sh", "-c", "echo pinned-bootstrap-proof >/result"]\n'
    )
    (fixture / "fixture").write_text("external-fixture\n")
    disk(work / "fixture.ext4", 16 * 1024**2, fixture)
    disk(work / "builder-cache.ext4", 8 * 1024**3)
    disk(work / "oci.ext4", 64 * 1024**2)
    results = [
        boot(
            work,
            firecracker,
            "builder",
            work / "builder-cache.ext4",
            work / "fixture.ext4",
            work / "oci.ext4",
        )
    ]
    disk(work / "converter-work.ext4", 3 * 1024**3)
    golden = Path(sys.argv[3]).resolve()
    golden_sha = "3376d10f5302599a8855a20a35a4ffb1db039b1c53d3a8d9cd7507fd84c9c066"
    assert golden.stat().st_size == 67108864 and bootstrap.digest(golden) == golden_sha
    results.append(
        boot(
            work,
            firecracker,
            "converter",
            work / "converter-work.ext4",
            golden,
        )
    )
    assert bootstrap.digest(work / "root.ext4") == manifest["root"]["sha256"]
    report = {
        "results": results,
        "root_sha256": manifest["root"]["sha256"],
        "fixture_busybox_sha256": bootstrap.digest(fixture / "probe"),
        "golden_input_sha256": golden_sha,
        "converted_gzip_sha256": bootstrap.digest(work / "converted.ext4.gz"),
        "production_jail": False,
        "network_devices": [],
    }
    (work / "proof.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
