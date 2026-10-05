#!/usr/bin/env python3
"""Resolve and verify bundled image tools after their seven-day waiting period."""

import base64
import datetime
import hashlib
import json
import os
import re
import sys
import urllib.request


def fetch(url, github=False):
    headers = {"Accept": "application/vnd.github+json"} if github else {}
    if github and os.environ.get("GH_TOKEN"):
        headers["Authorization"] = "Bearer " + os.environ["GH_TOKEN"]
    with urllib.request.urlopen(
        urllib.request.Request(url, headers=headers), timeout=60
    ) as response:
        return response.read()


def version(value, prefix=""):
    if not isinstance(value, str) or not value.startswith(prefix):
        return None
    value = value[len(prefix) :]
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", value):
        return None
    return tuple(map(int, value.split(".")))


def old_enough(value, cutoff):
    if not isinstance(value, str):
        return False
    try:
        published = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return False
    return published.tzinfo is not None and published <= cutoff


def release_old_enough(release, cutoff, asset_names):
    assets = {asset["name"]: asset for asset in release.get("assets", [])}
    return (
        not release.get("draft", True)
        and not release.get("prerelease", True)
        and old_enough(release.get("published_at"), cutoff)
        # Replacing an asset restarts its waiting period.
        and all(
            name in assets
            and old_enough(assets[name].get("created_at"), cutoff)
            and old_enough(assets[name].get("updated_at"), cutoff)
            for name in asset_names
        )
    )


def github_release(repo, prefix, current, cutoff, asset_names):
    latest = json.loads(
        fetch(f"https://api.github.com/repos/{repo}/releases/latest", github=True)
    )
    ceiling = version(latest.get("tag_name"), prefix)
    if ceiling is None or latest.get("draft", True) or latest.get("prerelease", True):
        raise ValueError(f"{repo}: invalid latest stable release")
    if ceiling <= current:
        return None
    if release_old_enough(latest, cutoff, asset_names):
        return latest
    eligible = []
    page = 1
    while True:
        releases = json.loads(
            fetch(
                f"https://api.github.com/repos/{repo}/releases?per_page=100&page={page}",
                github=True,
            )
        )
        if not isinstance(releases, list):
            raise ValueError(f"{repo}: invalid release list")
        for release in releases:
            number = version(release.get("tag_name"), prefix)
            if (
                number is not None
                and current < number <= ceiling
                and release_old_enough(release, cutoff, asset_names)
            ):
                eligible.append((number, release))
        if len(releases) < 100:
            break
        page += 1
    return max(eligible, key=lambda item: item[0])[1] if eligible else None


def npm_packages(names, current, cutoff):
    packages = [
        json.loads(fetch(f"https://registry.npmjs.org/{name}")) for name in names
    ]
    ceiling = version(packages[0].get("dist-tags", {}).get("latest"))
    if ceiling is None:
        raise ValueError(f"{names[0]}: invalid latest stable version")
    if ceiling <= current:
        return None
    eligible = []
    for candidate in packages[0]["versions"]:
        number = version(candidate)
        if number is None or not current < number <= ceiling:
            continue
        if all(
            candidate in package["versions"]
            and not package["versions"][candidate].get("deprecated")
            and old_enough(package.get("time", {}).get(candidate), cutoff)
            for package in packages
        ):
            eligible.append((number, candidate))
    if not eligible:
        return None
    selected = max(eligible)[1]
    manifests = [package["versions"][selected] for package in packages]
    if len(names) > 1:
        dependencies = manifests[0].get("optionalDependencies", {})
        if any(dependencies.get(name) != selected for name in names[1:]):
            raise ValueError("AnyDoc's Linux packages do not match its version")
    return manifests


def verified_pin(url, destination, digest, npm=False):
    if npm:
        if not url.startswith("https://registry.npmjs.org/"):
            raise ValueError("npm tarball is outside registry.npmjs.org")
        # Require a strong registry integrity hash; never fall back to npm's SHA-1.
        checks = re.findall(r"(?:^|\s)(sha(?:256|384|512))-([^\s]+)", digest or "")
        if not checks:
            raise ValueError(f"{destination}: missing npm integrity")
        algorithm, expected = max(checks, key=lambda item: int(item[0][3:]))
        expected = base64.b64decode(expected, validate=True)
    else:
        if not url.startswith("https://github.com/"):
            raise ValueError("release asset is outside github.com")
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest or ""):
            raise ValueError(f"{destination}: missing publisher SHA-256")
        algorithm, expected = "sha256", bytes.fromhex(digest.removeprefix("sha256:"))
    data = fetch(url)
    if hashlib.new(algorithm, data).digest() != expected:
        raise ValueError(f"{destination}: download does not match publisher integrity")
    return (
        f"ADD --checksum=sha256:{hashlib.sha256(data).hexdigest()} {url} {destination}"
    )


def main():
    with open(sys.argv[1]) as source:
        block = source.read().split("# >>> pins\n", 1)[1].split("# <<< pins", 1)[0]
    rows = {}
    for line in block.splitlines():
        match = re.fullmatch(r"ADD --checksum=sha256:[0-9a-f]{64} (\S+) (\S+)", line)
        if not match or match[2] in rows:
            raise ValueError("invalid or duplicate image pin")
        rows[match[2]] = (match[1], line)
    cutoff = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(days=7)
    for repo, prefix, paths in (
        (
            "oven-sh/bun",
            "bun-v",
            ["/tmp/bun-aarch64.zip", "/tmp/bun-x64.zip"],
        ),
        (
            "rtk-ai/rtk",
            "v",
            ["/tmp/rtk-aarch64.tar.gz", "/tmp/rtk-x64.tar.gz"],
        ),
    ):
        tags = {
            rows[path][0].split("/download/", 1)[1].split("/", 1)[0] for path in paths
        }
        if len(tags) != 1 or (current := version(tags.pop(), prefix)) is None:
            raise ValueError(f"{repo}: invalid existing pins")
        release = github_release(
            repo,
            prefix,
            current,
            cutoff,
            [rows[path][0].rsplit("/", 1)[1] for path in paths],
        )
        if release:
            assets = {asset["name"]: asset for asset in release["assets"]}
            for path in paths:
                asset = assets[rows[path][0].rsplit("/", 1)[1]]
                rows[path] = (
                    asset["browser_download_url"],
                    verified_pin(
                        asset["browser_download_url"], path, asset.get("digest")
                    ),
                )
    for names, paths in (
        (["@dietrichgebert/ponytail"], ["/tmp/ponytail.tgz"]),
        (
            [
                "@firecrawl/anydoc",
                "@firecrawl/anydoc-linux-arm64-gnu",
                "@firecrawl/anydoc-linux-x64-gnu",
            ],
            ["/tmp/anydoc.tgz", "/tmp/anydoc-arm64.tgz", "/tmp/anydoc-x64.tgz"],
        ),
    ):
        versions = {
            rows[path][0].rsplit("/-/", 1)[1].removesuffix(".tgz").rsplit("-", 1)[1]
            for path in paths
        }
        if len(versions) != 1 or (current := version(versions.pop())) is None:
            raise ValueError(f"{names[0]}: invalid existing pins")
        manifests = npm_packages(names, current, cutoff)
        if manifests:
            for manifest, path in zip(manifests, paths, strict=True):
                dist = manifest["dist"]
                rows[path] = (
                    dist["tarball"],
                    verified_pin(
                        dist["tarball"], path, dist.get("integrity"), npm=True
                    ),
                )
    # Emit only after every source and download succeeded; the caller stages both files.
    for _, line in rows.values():
        print(line)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"bump-pins: {error}", file=sys.stderr)
        sys.exit(1)
