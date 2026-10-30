#!/usr/bin/env python3
"""Build the Linux builder/converter bootstrap without an installed builder."""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
LIMIT = 512 * 1024 * 1024


def run(*args, capture=False, cwd=None, env=None):
    return subprocess.run(
        [str(arg) for arg in args],
        check=True,
        stdout=subprocess.PIPE if capture else None,
        text=True,
        cwd=cwd,
        env=env,
    ).stdout


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def fetch(url, target, expected):
    target.parent.mkdir(parents=True, exist_ok=True)
    # curl >= 8.4 also enforces max-filesize without Content-Length.
    run(
        "curl",
        "--disable",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "15",
        "--max-time",
        "300",
        "--max-filesize",
        str(LIMIT),
        "--speed-limit",
        "1",
        "--speed-time",
        "30",
        "--output",
        target,
        url,
    )
    if target.stat().st_size > LIMIT or digest(target) != expected:
        raise ValueError(f"checksum/size mismatch: {url}")


def extract_source(archive, directory, prefix):
    if not hasattr(tarfile, "data_filter"):
        raise ValueError("Python tarfile.data_filter is required for source extraction")
    directory.mkdir()
    with tarfile.open(archive) as source:
        members = source.getmembers()
        if sum(member.size for member in members) > LIMIT:
            raise ValueError("expanded producer source exceeds 512 MiB")
        for member in members:
            parts = member.name.split("/")
            if parts[0] != prefix or ".." in parts:
                raise ValueError("producer source has an unexpected archive path")
        # The stdlib data filter also checks link targets and symlink ancestors,
        # rejects devices, strips special bits and ignores archived ownership.
        source.extractall(directory, members=members, filter="data")
    return directory / prefix


def discovery_helper(pins, work, root, expected_source):
    go_pin, bk = pins["go"], pins["buildkit"]
    if not re.fullmatch("[0-9a-f]{40}", bk["commit"]):
        raise ValueError("BuildKit source commit must be an exact lowercase object id")
    go_archive = work / f"go{go_pin['version']}.linux-amd64.tar.gz"
    source_archive = work / f"buildkit-source-{bk['commit']}.tar.gz"
    fetch(go_pin["url"], go_archive, go_pin["sha256"])
    fetch(bk["source_url"], source_archive, bk["source_sha256"])
    go = extract_source(go_archive, work / "go-source", "go")
    source = extract_source(
        source_archive, work / "buildkit-source", f"buildkit-{bk['commit']}"
    )
    for name, expected in bk["source_files"].items():
        if digest(source / name) != expected:
            raise ValueError(f"BuildKit producer input checksum mismatch: {name}")
    package = source / "cmd/pinfold-discover"
    package.mkdir()
    helper_source = package / "main.go"
    shutil.copyfile(HERE / "discover/main.go", helper_source)
    if digest(helper_source) != expected_source:
        raise ValueError("discovery helper source changed during bootstrap preparation")
    environment = {
        "PATH": f"{go / 'bin'}:/usr/bin:/bin",
        "TMPDIR": str(work / "go-tmp"),
        "GOCACHE": str(work / "go-cache"),
        "GOMODCACHE": str(work / "go-modcache"),
        "GOPATH": str(work / "go-path"),
        "GOENV": "off",
        "GOWORK": "off",
        "GOTOOLCHAIN": "local",
        "GOPROXY": "off",
        "GOSUMDB": "off",
        "CGO_ENABLED": "0",
        "GOOS": "linux",
        "GOARCH": "amd64",
    }
    for name in ["TMPDIR", "GOCACHE", "GOMODCACHE", "GOPATH"]:
        Path(environment[name]).mkdir()
    compiler = run(go / "bin/go", "version", capture=True, env=environment).strip()
    if compiler != f"go version go{go_pin['version']} linux/amd64":
        raise ValueError("downloaded Go compiler version differs from its pin")
    binary = root / "usr/local/bin/pinfold-discover"
    command = [
        str(go / "bin/go"),
        "build",
        "-mod=vendor",
        "-trimpath",
        "-buildvcs=false",
        "-ldflags",
        f"-s -w -X github.com/moby/buildkit/version.Version=v{bk['version']} "
        f"-X github.com/moby/buildkit/version.Revision={bk['commit']}",
        "-o",
        str(binary),
        "./cmd/pinfold-discover",
    ]
    run(*command, cwd=source, env=environment)
    binary.chmod(0o755)
    info = {
        "source_sha256": expected_source,
        "buildkit_commit": bk["commit"],
        "buildkit_source_sha256": bk["source_sha256"],
        "source_files": bk["source_files"],
        "compiler": compiler,
        "toolchain_sha256": go_pin["sha256"],
        "binary": {
            "path": "/usr/local/bin/pinfold-discover",
            "bytes": binary.stat().st_size,
            "sha256": digest(binary),
        },
        "command": command,
        "environment": environment,
        "build_info": run(
            go / "bin/go", "version", "-m", binary, capture=True, env=environment
        ),
    }
    return info, helper_source, [go_archive, source_archive]


def build(out, work):
    pins = json.loads((HERE / "pins.json").read_text())
    sources = {
        name: digest(HERE / name)
        for name in [
            "build.py",
            "pins.json",
            "packages.tsv",
            "debian-archive.asc",
            "discover/main.go",
        ]
    }
    recipe = hashlib.sha256(json.dumps(sources, sort_keys=True).encode()).hexdigest()
    if sources["debian-archive.asc"] != pins["keyring_sha256"]:
        raise ValueError("Debian signing key checksum mismatch")
    keyring = work / "debian.gpg"
    run("gpg", "--batch", "--dearmor", "--output", keyring, HERE / "debian-archive.asc")
    mirror = work / "mirror"
    suite = mirror / "dists/trixie"
    for name, key in [
        ("Release", "release_sha256"),
        ("Release.gpg", "release_signature_sha256"),
    ]:
        fetch(f"{pins['snapshot']}/dists/trixie/{name}", suite / name, pins[key])
    run("gpgv", "--keyring", keyring, suite / "Release.gpg", suite / "Release")
    index = "main/binary-amd64/Packages.xz"
    # Tie the pinned index to the authenticated Release as well as our pins.
    if not re.search(
        rf"^ {pins['packages_sha256']} +[0-9]+ {re.escape(index)}$",
        (suite / "Release").read_text(),
        re.MULTILINE,
    ):
        raise ValueError("package index absent from signed Release")
    fetch(
        f"{pins['snapshot']}/dists/trixie/{index}",
        suite / index,
        pins["packages_sha256"],
    )
    packages = [
        line.split("\t") for line in (HERE / "packages.tsv").read_text().splitlines()
    ]
    for name, version, arch, sha, path in packages:
        fetch(f"{pins['snapshot']}/{path}", mirror / path, sha)

    root = work / "root"
    # A file mirror makes debootstrap offline. Missing or extra dependencies fail
    # instead of resolving packages from another date or a live apt repository.
    run(
        "/usr/sbin/debootstrap",
        "--arch=amd64",
        "--variant=minbase",
        "--include=ca-certificates,e2fsprogs,util-linux",
        "--force-check-sig",
        f"--keyring={keyring}",
        "trixie",
        root,
        f"file://{mirror}",
    )
    installed = run(
        "chroot",
        root,
        "dpkg-query",
        "-W",
        "-f=${Package}\t${Version}\t${Architecture}\n",
        capture=True,
    )
    expected = "".join("\t".join(row[:3]) + "\n" for row in packages)
    if "\n".join(sorted(installed.splitlines())) + "\n" != expected:
        raise ValueError("installed Debian package closure differs from packages.tsv")

    bk = pins["buildkit"]
    archive = work / "buildkit.tar.gz"
    fetch(bk["url"], archive, bk["sha256"])
    with tarfile.open(archive) as source:
        (root / "usr/local/bin").mkdir(parents=True, exist_ok=True)
        for name in ["buildctl", "buildkitd", "buildkit-runc"]:
            member = source.getmember(f"bin/{name}")
            if not member.isfile():
                raise ValueError(f"BuildKit {name} is not a regular file")
            target = root / "usr/local/bin" / name
            with source.extractfile(member) as reader, target.open("wb") as writer:
                shutil.copyfileobj(reader, writer)
            target.chmod(0o755)
    helper, helper_source, producer_archives = discovery_helper(
        pins, work, root, sources["discover/main.go"]
    )
    for path in ["debootstrap", "var/lib/apt/lists", "var/cache/apt", "var/log"]:
        shutil.rmtree(root / path, ignore_errors=True)
        if path != "debootstrap":
            (root / path).mkdir(parents=True, exist_ok=True)
    for path in ["etc/resolv.conf", "etc/hostname", "etc/machine-id"]:
        (root / path).write_bytes(b"")
    (root / "etc/apt/sources.list").write_bytes(b"")
    for path in [
        "run",
        "tmp",
        "proc",
        "sys",
        "dev",
        "input",
        "work",
        "var/lib/buildkit",
    ]:
        (root / path).mkdir(parents=True, exist_ok=True)

    stage = work / "dist"
    stage.mkdir()
    disk = stage / "root.ext4"
    # This is the trial's tested bootstrap size, not a build/cache/output limit.
    with disk.open("wb") as target:
        target.truncate(512 * 1024 * 1024)
    run("/usr/sbin/mkfs.ext4", "-q", "-F", "-d", root, disk)
    root_info = {"bytes": disk.stat().st_size, "sha256": digest(disk)}
    asset = stage / "bootstrap.tar.gz"
    run(
        "tar",
        "--sort=name",
        "--mtime=@0",
        "--owner=0",
        "--group=0",
        "--numeric-owner",
        "-czf",
        asset,
        "-C",
        stage,
        "root.ext4",
    )
    if asset.stat().st_size > LIMIT:
        raise ValueError("compressed bootstrap exceeds the artifact download cap")
    artifact_sha = digest(asset)
    asset = asset.rename(stage / f"pinfold-builder-linux-x64-{artifact_sha}.tar.gz")
    manifest = {
        "recipe_sha256": recipe,
        "sources": sources,
        "pins": pins,
        "debian_packages": len(packages),
        "root": root_info,
        "artifact": {
            "name": asset.name,
            "bytes": asset.stat().st_size,
            "sha256": artifact_sha,
        },
        "bit_identical_filesystem": False,
        "discovery_helper": helper,
        "tools": {
            "python": sys.version,
            "debootstrap": run("dpkg-query", "-W", "debootstrap", capture=True).strip(),
            "e2fsprogs": run("dpkg-query", "-W", "e2fsprogs", capture=True).strip(),
            "curl": run("curl", "--version", capture=True).splitlines()[0],
            "gpgv": run("gpgv", "--version", capture=True).splitlines()[0],
            "tar": run("tar", "--version", capture=True).splitlines()[0],
        },
    }
    disk.unlink()
    (stage / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    for name in ["packages.tsv", "pins.json", "debian-archive.asc"]:
        shutil.copyfile(HERE / name, stage / name)
    for name in ["Release", "Release.gpg"]:
        shutil.copyfile(suite / name, stage / name)
    shutil.copyfile(helper_source, stage / "discovery-main.go")
    (stage / "discovery-build.json").write_text(
        json.dumps(helper, indent=2, sort_keys=True) + "\n"
    )
    for archive in producer_archives:
        shutil.copyfile(archive, stage / archive.name)
    (stage / "SHA256SUMS").write_text(
        "".join(f"{digest(path)}  {path.name}\n" for path in sorted(stage.iterdir()))
    )
    stage.rename(out)
    print(json.dumps(manifest, sort_keys=True))


if __name__ == "__main__":
    if (
        len(sys.argv) != 2
        or os.geteuid() != 0
        or sys.platform != "linux"
        or os.uname().machine != "x86_64"
    ):
        sys.exit(
            "usage: sudo python3 builder-root/build.py NEW_OUTPUT_DIR (Linux x86_64)"
        )
    curl = tuple(map(int, run("curl", "--version", capture=True).split()[1].split(".")))
    if curl < (8, 4, 0):
        sys.exit("curl >= 8.4.0 is required for streaming download bounds")
    output = Path(sys.argv[1]).resolve()
    if output.exists():
        sys.exit("output directory already exists")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="builder-root-", dir=output.parent
    ) as temporary:
        build(output, Path(temporary))
