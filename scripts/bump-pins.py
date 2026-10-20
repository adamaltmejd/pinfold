#!/usr/bin/env python3
"""Repin repository dependencies and tools (ARCHITECTURE.md, Pinned artifacts)."""

import base64
import datetime
import hashlib
import json
import os
import re
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMAGE = ROOT / "profile" / "image"
HARNESSES = ROOT / "crates" / "pinfold" / "harnesses.toml"
VMM = ROOT / "crates" / "pinfold" / "vmm.toml"
KERNEL = ROOT / "guest-kernel" / "pins"
YARD = ROOT / ".yard" / "Dockerfile"
# Containerfile pins that move together, by ADD destination. AnyDoc's CLI comes
# first: its optionalDependencies must name its native packages' version.
IMAGE_TOOLS = {
    "bun": (IMAGE / "bun.Containerfile", ["/tmp/bun-aarch64.zip", "/tmp/bun-x64.zip"]),
    "rtk": (
        IMAGE / "full.Containerfile",
        ["/tmp/rtk-aarch64.tar.gz", "/tmp/rtk-x64.tar.gz"],
    ),
    "ponytail": (IMAGE / "full.Containerfile", ["/tmp/ponytail.tgz"]),
    "anydoc": (
        IMAGE / "documents.Containerfile",
        ["/tmp/anydoc.tgz", "/tmp/anydoc-aarch64.tgz", "/tmp/anydoc-x64.tgz"],
    ),
}
WAIT = datetime.timedelta(days=7)
VERSION = r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)"
CLAUDE = "https://downloads.claude.ai/claude-code-releases/"


def fetch(url, github=False):
    headers = {
        "User-Agent": "pinfold-pin-updater (https://github.com/adamaltmejd/pinfold)"
    }
    if github:
        headers["Accept"] = "application/vnd.github+json"
        if os.environ.get("GH_TOKEN"):
            headers["Authorization"] = "Bearer " + os.environ["GH_TOKEN"]
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def version(text):
    if not isinstance(text, str) or not re.fullmatch(VERSION, text):
        return None
    return tuple(map(int, text.split(".")))


def old_enough(value, cutoff):
    """Whether timestamp `value` is at or before `cutoff`; None waits for nothing."""
    if cutoff is None:
        return True
    try:
        published = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except (AttributeError, ValueError):
        return False
    return published.tzinfo is not None and published <= cutoff


def verified(url, algorithm, expected):
    """The sha256 of `url`'s bytes, which must hash to the publisher's digest."""
    data = fetch(url)
    if hashlib.new(algorithm, data).digest() != expected:
        raise ValueError(f"{url}: download does not match its publisher's digest")
    return hashlib.sha256(data).hexdigest()


# Each source takes one tool's pinned urls and returns None when they are
# current, else (current, target, rows): rows are the new (url, sha256) in the
# same order, or None while no version above current is eligible.


def github(urls, cutoff):
    parts = [
        re.fullmatch(
            rf"https://github\.com/([^/]+/[^/]+)/releases/download/(.*?)({VERSION})/([^/]+)",
            url,
        )
        for url in urls
    ]
    if not all(parts) or len({part.group(1, 2, 3) for part in parts}) != 1:
        raise ValueError(f"{urls[0]}: pins are not one GitHub release")
    repo, prefix, current = parts[0][1], parts[0][2], version(parts[0][3])
    # TruffleHog includes its version in asset names.
    names = [part[4] for part in parts]

    def asset_names(number):
        target = ".".join(map(str, number))
        return [name.replace(parts[0][3], target) for name in names]

    def number(release):
        tag = release.get("tag_name")
        if not isinstance(tag, str) or not tag.startswith(prefix):
            return None
        return version(tag.removeprefix(prefix))

    def complete(release):
        assets = {asset["name"]: asset for asset in release.get("assets", [])}
        return (
            not release.get("draft", True)
            and not release.get("prerelease", True)
            and old_enough(release.get("published_at"), cutoff)
            # Replacing an asset restarts its wait.
            and all(
                name in assets
                and old_enough(assets[name].get("created_at"), cutoff)
                and old_enough(assets[name].get("updated_at"), cutoff)
                for name in asset_names(number(release))
            )
        )

    api = f"https://api.github.com/repos/{repo}/releases"
    latest = json.loads(fetch(f"{api}/latest", github=True))
    ceiling = number(latest)
    if ceiling is None:
        raise ValueError(f"{repo}: latest release is not stable")
    if ceiling <= current:
        return None
    releases = [latest]
    # Harnesses take only the latest release; a waiting tool may take an older one.
    if cutoff is not None and not complete(latest):
        page = 1
        while True:
            batch = json.loads(fetch(f"{api}?per_page=100&page={page}", github=True))
            releases += batch
            if len(batch) < 100:
                break
            page += 1
    eligible = [
        (number(release), release)
        for release in releases
        if number(release) is not None
        and current < number(release) <= ceiling
        and complete(release)
    ]
    if not eligible:
        return current, ceiling, None
    target, release = max(eligible, key=lambda item: item[0])
    assets = {asset["name"]: asset for asset in release["assets"]}
    rows = []
    for name in asset_names(target):
        url, digest = assets[name]["browser_download_url"], assets[name].get("digest")
        if not url.startswith("https://github.com/"):
            raise ValueError(f"{url}: asset is outside github.com")
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest or ""):
            raise ValueError(f"{url}: no publisher SHA-256")
        rows.append((url, verified(url, "sha256", bytes.fromhex(digest[7:]))))
    return current, target, rows


def npm(urls, cutoff):
    parts = [
        re.fullmatch(
            rf"https://registry\.npmjs\.org/((?:@[^/]+/)?[^/]+)/-/[^/]+-({VERSION})\.tgz",
            url,
        )
        for url in urls
    ]
    if not all(parts) or len({part[2] for part in parts}) != 1:
        raise ValueError(f"{urls[0]}: pins are not one npm version")
    names, current = [part[1] for part in parts], version(parts[0][2])
    packages = [
        json.loads(fetch(f"https://registry.npmjs.org/{name}")) for name in names
    ]
    ceiling = version(packages[0].get("dist-tags", {}).get("latest"))
    if ceiling is None:
        raise ValueError(f"{names[0]}: latest version is not stable")
    if ceiling <= current:
        return None

    def complete(candidate):
        manifest = packages[0]["versions"][candidate]
        natives = manifest.get("optionalDependencies", {})
        return all(natives.get(name) == candidate for name in names[1:]) and all(
            candidate in package["versions"]
            and not package["versions"][candidate].get("deprecated")
            and old_enough(package.get("time", {}).get(candidate), cutoff)
            for package in packages
        )

    eligible = [
        (number, candidate)
        for candidate in packages[0]["versions"]
        if (number := version(candidate)) is not None
        and current < number <= ceiling
        and complete(candidate)
    ]
    if not eligible:
        return current, ceiling, None
    target, candidate = max(eligible)
    rows = []
    for package in packages:
        dist = package["versions"][candidate]["dist"]
        url = dist["tarball"]
        if not url.startswith("https://registry.npmjs.org/"):
            raise ValueError(f"{url}: tarball is outside registry.npmjs.org")
        # The strongest integrity hash; never npm's legacy SHA-1.
        checks = re.findall(
            r"(?:^|\s)sha(256|384|512)-(\S+)", dist.get("integrity", "")
        )
        if not checks:
            raise ValueError(f"{url}: no strong npm integrity")
        bits, digest = max(checks, key=lambda check: int(check[0]))
        rows.append(
            (url, verified(url, f"sha{bits}", base64.b64decode(digest, validate=True)))
        )
    return current, target, rows


def claude(urls, cutoff):
    parts = [
        re.fullmatch(rf"{re.escape(CLAUDE)}({VERSION})/([^/]+)/claude", url)
        for url in urls
    ]
    if not all(parts) or len({part[1] for part in parts}) != 1:
        raise ValueError(f"{urls[0]}: pins are not one claude release")
    current = version(parts[0][1])
    latest = fetch(CLAUDE + "latest").decode().strip()
    target = version(latest)
    if target is None:
        raise ValueError(f"claude: latest version {latest!r} is not stable")
    if target <= current:
        return None
    platforms = json.loads(fetch(f"{CLAUDE}{latest}/manifest.json"))["platforms"]
    if not all(part[2] in platforms for part in parts):
        return current, target, None
    rows = []
    for part in parts:
        url = f"{CLAUDE}{latest}/{part[2]}/claude"
        expected = bytes.fromhex(platforms[part[2]]["checksum"])
        rows.append((url, verified(url, "sha256", expected)))
    return current, target, rows


SOURCES = [
    ("https://github.com/", github),
    ("https://registry.npmjs.org/", npm),
    (CLAUDE, claude),
]


def replace(text, old, new):
    if text.count(old) != 1:
        raise ValueError(f"{old!r} is not exactly once in its pin file")
    return text.replace(old, new)


def repin_firecracker(texts, cutoff):
    """Move Firecracker in vmm.toml with the guest kernel release built for it
    and the guest config pin it implies. The release must already exist: the
    guest-kernel workflow publishes it, never this script."""
    vmm = tomllib.loads(texts[VMM])
    firecracker, kernel = vmm["asset"]
    current_firecracker, current_kernel = vmm["version"].split("+")
    pinned = re.search(r"^KERNEL=(\S+)$", texts[KERNEL], re.MULTILINE)[1]
    if pinned != current_kernel:
        raise ValueError(f"vmm.toml's kernel {current_kernel} is not KERNEL={pinned}")
    update = github([firecracker["url"]], cutoff)
    if update is None:
        return
    current, target, rows = update
    current, target = ".".join(map(str, current)), ".".join(map(str, target))
    if rows is None:
        print(f"firecracker {current}: {target} is not yet eligible")
        return
    tag = f"guest-kernel-{pinned}-firecracker-v{target}"
    name = f"vmlinux-{pinned}"
    try:
        release = json.loads(
            fetch(
                f"https://api.github.com/repos/adamaltmejd/pinfold/releases/tags/{tag}",
                github=True,
            )
        )
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        print(f"firecracker {current}: {target} awaits the guest kernel release {tag}")
        return
    assets = {asset["name"]: asset for asset in release.get("assets", [])}
    if name not in assets:
        raise ValueError(f"{tag}: no {name}")
    kernel_url, digest = (
        assets[name]["browser_download_url"],
        assets[name].get("digest"),
    )
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest or ""):
        raise ValueError(f"{kernel_url}: no publisher SHA-256")
    kernel_sha = verified(kernel_url, "sha256", bytes.fromhex(digest[7:]))
    [(url, sha)] = rows
    config = fetch(
        "https://raw.githubusercontent.com/firecracker-microvm/firecracker/"
        f"v{target}/resources/guest_configs/"
        f"microvm-kernel-ci-x86_64-{pinned.rsplit('.', 1)[0]}.config"
    )
    old = re.search(r"^CONFIG_SHA256=[0-9a-f]{64}$", texts[KERNEL], re.MULTILINE)[0]
    print(f"firecracker {current} -> {target}, guest kernel -> {tag}")
    texts[KERNEL] = replace(
        texts[KERNEL], old, f"CONFIG_SHA256={hashlib.sha256(config).hexdigest()}"
    )
    for old, new in [
        (firecracker["url"], url),
        (firecracker["sha256"], sha),
        (kernel["url"], kernel_url),
        (kernel["sha256"], kernel_sha),
        (f'version = "{current_firecracker}+', f'version = "{target}+'),
    ]:
        texts[VMM] = replace(texts[VMM], old, new)


def repin_build_tools(texts):
    path = ROOT / "rust-toolchain.toml"
    current = tomllib.loads(texts[path])["toolchain"]["channel"]
    manifest = tomllib.loads(
        fetch("https://static.rust-lang.org/dist/channel-rust-stable.toml").decode()
    )
    target = manifest["pkg"]["rust"]["version"].split()[0]
    if version(current) is None or version(target) is None:
        raise ValueError("Rust toolchains must be stable versions")
    if version(target) > version(current):
        image = json.loads(
            fetch(
                f"https://hub.docker.com/v2/repositories/library/rust/tags/{target}-slim-trixie"
            )
        )
        if not isinstance(image["images"], list):
            raise ValueError(f"Rust {target}: invalid Yard image platform list")
        platforms = {(row["os"], row["architecture"]) for row in image["images"]}
        if any(
            not isinstance(value, str) or not value
            for row in platforms
            for value in row
        ):
            raise ValueError(f"Rust {target}: invalid Yard image platform")
        if not {("linux", "amd64"), ("linux", "arm64")} <= platforms:
            print(f"rust {current}: {target} awaits both Yard image architectures")
        else:
            print(f"rust {current} -> {target}")
            texts[path] = replace(
                texts[path], f'channel = "{current}"', f'channel = "{target}"'
            )
            texts[YARD] = replace(
                texts[YARD], f"rust:{current}-slim-trixie", f"rust:{target}-slim-trixie"
            )

    path = ROOT / "Cargo.toml"
    for name, dependency in tomllib.loads(texts[path])["workspace"][
        "dependencies"
    ].items():
        current = dependency if isinstance(dependency, str) else dependency["version"]
        metadata = json.loads(fetch(f"https://crates.io/api/v1/crates/{name}"))
        eligible = [
            (number, row["num"].split("+", 1)[0])
            for row in metadata["versions"]
            if (number := version(row["num"].split("+", 1)[0])) is not None
            and not row["yanked"]
        ]
        target = max(eligible)[1]
        # Requirements are deliberately raised across incompatible versions.
        if tuple(map(int, current.split("."))) >= version(target):
            continue
        pattern = (
            rf'(^\s*{re.escape(name)} = (?:\{{ version = )?"){re.escape(current)}(")'
        )
        texts[path], count = re.subn(
            pattern, lambda m: m[1] + target + m[2], texts[path], flags=re.MULTILINE
        )
        if count != 1:
            raise ValueError(f"{name}: cannot update Cargo requirement")
        print(f"{name} {current} -> {target}")


def repin_packages(texts, cutoff):
    path = ROOT / "examples" / "pi-durable" / "package.json"
    manifest = json.loads(texts[path])
    groups = [manifest[key] for key in ("dependencies", "devDependencies", "overrides")]
    packages = {
        name: json.loads(fetch(f"https://registry.npmjs.org/{name}"))
        for group in groups
        for name in group
    }

    def eligible(name):
        package = packages[name]
        ceiling = version(package["dist-tags"]["latest"])
        if ceiling is None:
            raise ValueError(f"{name}: latest version is not stable")
        return {
            number: candidate
            for candidate, row in package["versions"].items()
            if (number := version(candidate)) is not None
            and number <= ceiling
            and not row.get("deprecated")
            and old_enough(package.get("time", {}).get(candidate), cutoff)
        }

    versions = {name: eligible(name) for name in packages}
    for group in groups:
        for name, current in group.items():
            newer = [number for number in versions[name] if number > version(current)]
            if newer:
                target = versions[name][max(newer)]
                print(f"{name} {current} -> {target}")
                group[name] = target
    texts[path] = json.dumps(manifest, indent=2) + "\n"


def repin_yard(texts, cutoff):
    current = re.search(r"TRUFFLEHOG_VERSION=(" + VERSION + r")", texts[YARD])[1]
    rows = re.findall(r"arch=(arm64|amd64); sum=([0-9a-f]{64})", texts[YARD])
    if len(rows) != 2:
        raise ValueError("TruffleHog must pin both Linux architectures")
    urls = [
        f"https://github.com/trufflesecurity/trufflehog/releases/download/v{current}/trufflehog_{current}_linux_{arch}.tar.gz"
        for arch, _ in rows
    ]
    update = github(urls, cutoff)
    if update is None or update[2] is None:
        return
    _, target, assets = update
    target = ".".join(map(str, target))
    print(f"trufflehog {current} -> {target}")
    texts[YARD] = replace(
        texts[YARD], f"TRUFFLEHOG_VERSION={current}", f"TRUFFLEHOG_VERSION={target}"
    )
    for (_, old_sha), (_, sha) in zip(rows, assets, strict=True):
        texts[YARD] = replace(texts[YARD], old_sha, sha)


def repin_suffix_list(texts):
    path = ROOT / "crates" / "pinfold" / "public_suffix_list.dat"
    target = fetch("https://publicsuffix.org/list/public_suffix_list.dat").decode()
    pattern = r"^// VERSION: (\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}_UTC)$"
    current = re.search(pattern, texts[path], re.MULTILINE)
    new = re.search(pattern, target, re.MULTILINE)
    if (
        not current
        or not new
        or not re.search(r"^// COMMIT: [0-9a-f]{40}$", target, re.MULTILINE)
    ):
        raise ValueError("Public Suffix List lacks its version or commit")
    if not all(
        marker in target
        for marker in (
            "// ===BEGIN ICANN DOMAINS===",
            "// ===END ICANN DOMAINS===",
            "// ===BEGIN PRIVATE DOMAINS===",
            "// ===END PRIVATE DOMAINS===",
            "\ncom\n",
        )
    ):
        raise ValueError("Public Suffix List is incomplete")
    if new[1] > current[1]:
        print(f"public suffix list {current[1]} -> {new[1]}")
        texts[path] = target


def main():
    recipes = sorted({path for path, _ in IMAGE_TOOLS.values()})
    texts = {
        path: path.read_text()
        for path in [
            *recipes,
            HARNESSES,
            VMM,
            KERNEL,
            YARD,
            ROOT / "rust-toolchain.toml",
            ROOT / "Cargo.toml",
            ROOT / "examples" / "pi-durable" / "package.json",
            ROOT / "crates" / "pinfold" / "public_suffix_list.dat",
        ]
    }
    pins = {}
    for path in recipes:
        rows = re.findall(
            r"^ADD --checksum=sha256:([0-9a-f]{64}) (\S+) (\S+)$",
            texts[path],
            re.MULTILINE,
        )
        expected = [
            dest
            for source, dests in IMAGE_TOOLS.values()
            if source == path
            for dest in dests
        ]
        if sorted(dest for _, _, dest in rows) != sorted(expected):
            raise ValueError(f"{path}: ADD pins do not match IMAGE_TOOLS")
        pins.update({(path, dest): (url, sha) for sha, url, dest in rows})
    cutoff = datetime.datetime.now(datetime.timezone.utc) - WAIT
    tools = [
        (name, path, [pins[path, dest] for dest in dests], cutoff)
        for name, (path, dests) in IMAGE_TOOLS.items()
    ]
    for harness in tomllib.loads(texts[HARNESSES])["harness"]:
        rows = [(asset["url"], asset["sha256"]) for asset in harness["asset"]]
        tools.append((harness["name"], HARNESSES, rows, None))

    for name, path, rows, wait in tools:
        urls = [url for url, _ in rows]
        sources = [s for prefix, s in SOURCES if urls[0].startswith(prefix)]
        if not sources:
            raise ValueError(f"{urls[0]}: no upstream source")
        update = sources[0](urls, wait)
        if update is None:
            continue
        current, target, new_rows = update
        current, target = ".".join(map(str, current)), ".".join(map(str, target))
        if new_rows is None:
            # Also how a renamed or dropped asset shows; it never becomes eligible.
            print(f"{name} {current}: {target} is not yet eligible")
            continue
        print(f"{name} {current} -> {target}")
        for (old_url, old_sha), (url, sha) in zip(rows, new_rows, strict=True):
            texts[path] = replace(texts[path], old_url, url)
            texts[path] = replace(texts[path], old_sha, sha)
        if path == HARNESSES:
            texts[path] = replace(
                texts[path],
                f'name = "{name}"\nversion = "{current}"',
                f'name = "{name}"\nversion = "{target}"',
            )

    repin_firecracker(texts, cutoff)
    repin_build_tools(texts)
    repin_packages(texts, cutoff)
    repin_yard(texts, cutoff)
    repin_suffix_list(texts)
    for path, text in texts.items():
        path.write_text(text)


if __name__ == "__main__":
    main()
