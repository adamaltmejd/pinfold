#!/usr/bin/env python3
"""Repin harnesses.toml and the profile recipe ADD pins (ARCHITECTURE.md, Pinned artifacts)."""

import base64
import datetime
import hashlib
import json
import os
import re
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMAGE = ROOT / "profile" / "image"
HARNESSES = ROOT / "crates" / "pinfold" / "harnesses.toml"
# Containerfile pins that move together, by ADD destination. AnyDoc's CLI comes
# first: its optionalDependencies must name its native packages' version.
IMAGE_TOOLS = {
    "bun": (IMAGE / "bun.Containerfile", ["/tmp/bun-aarch64.zip", "/tmp/bun-x64.zip"]),
    "rtk": (IMAGE / "full.Containerfile", ["/tmp/rtk-aarch64.tar.gz", "/tmp/rtk-x64.tar.gz"]),
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
    headers = {}
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
    names = [part[4] for part in parts]

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
                for name in names
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
    for name in names:
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


def main():
    recipes = sorted({path for path, _ in IMAGE_TOOLS.values()})
    texts = {path: path.read_text() for path in [*recipes, HARNESSES]}
    pins = {}
    for path in recipes:
        rows = re.findall(
            r"^ADD --checksum=sha256:([0-9a-f]{64}) (\S+) (\S+)$",
            texts[path],
            re.MULTILINE,
        )
        expected = [
            dest for source, dests in IMAGE_TOOLS.values() if source == path for dest in dests
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

    for path, text in texts.items():
        path.write_text(text)


if __name__ == "__main__":
    main()
