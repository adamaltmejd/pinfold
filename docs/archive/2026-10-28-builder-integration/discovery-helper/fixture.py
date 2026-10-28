#!/usr/bin/env python3
"""Loopback registry fixture/comparator, not a Pinfold security gate.

Spawns no daemon or helper, executes no recipe, and makes no outbound request.
Use only on the leased disposable host. The parent supervises the owned pinned
BuildKit daemon and invokes native conversion subrequests without exporters.
"""

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import io
import json
import os
from pathlib import Path
import signal
import tarfile
from urllib.parse import urlsplit

IMAGE = "application/vnd.oci.image.manifest.v1+json"
CONFIG = "application/vnd.oci.image.config.v1+json"
LAYER = "application/vnd.oci.image.layer.v1.tar"
LIMIT = 1024 * 1024


def require(value, detail):
    if not value:
        raise ValueError(detail)


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def write(path, data):
    with os.fdopen(
        os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb"
    ) as output:
        output.write(data)


def json_write(path, value):
    write(path, encoded(value) + b"\n")


def images(work):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        marker = b"discovery-layer-marker\n"
        entry = tarfile.TarInfo("fixture-marker")
        entry.mode, entry.uid, entry.gid, entry.mtime, entry.size = (
            0o644,
            0,
            0,
            0,
            len(marker),
        )
        archive.addfile(entry, io.BytesIO(marker))
    layer = stream.getvalue()
    result = {}
    for name in ["zero", "layer"]:
        directory = work / name
        directory.mkdir(mode=0o700)
        config = {
            "architecture": "amd64",
            "os": "linux",
            "config": {"Env": [f"DISCOVERY_FIXTURE={name}"]},
            "rootfs": {"type": "layers", "diff_ids": []},
            "history": [],
        }
        layers, blobs = [], {}
        if name == "zero":
            config["config"]["OnBuild"] = ["RUN false"]
        else:
            config["rootfs"]["diff_ids"] = [digest(layer)]
            config["history"] = [{"created_by": "independent registry marker fixture"}]
            layers = [{"mediaType": LAYER, "digest": digest(layer), "size": len(layer)}]
            blobs[digest(layer)] = layer
        config = encoded(config)
        blobs[digest(config)] = config
        manifest = encoded(
            {
                "schemaVersion": 2,
                "mediaType": IMAGE,
                "config": {
                    "mediaType": CONFIG,
                    "digest": digest(config),
                    "size": len(config),
                },
                "layers": layers,
            }
        )
        write(directory / "manifest.json", manifest)
        for identity, data in blobs.items():
            write(directory / identity.removeprefix("sha256:"), data)
        result[f"fixture/{name}"] = {
            "manifest": manifest,
            "digest": digest(manifest),
            "blobs": blobs,
        }
    return result


def serve(work):
    work.mkdir(mode=0o700)
    records = images(work)
    request_log = os.fdopen(
        os.open(work / "requests.jsonl", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600),
        "wb",
    )

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def log_message(self, *_):
            pass

        def reply(self, head):
            path = urlsplit(self.path).path
            request_log.write(encoded({"method": self.command, "path": path}) + b"\n")
            request_log.flush()
            data, media, identity, status = b"{}", "application/json", None, 404
            if path in ["/v2", "/v2/"]:
                status = 200
            for repository, image in records.items():
                manifest_prefix = f"/v2/{repository}/manifests/"
                blob_prefix = f"/v2/{repository}/blobs/"
                if path.startswith(manifest_prefix) and path[
                    len(manifest_prefix) :
                ] in ["latest", image["digest"]]:
                    data, media, identity, status = (
                        image["manifest"],
                        IMAGE,
                        image["digest"],
                        200,
                    )
                elif (
                    path.startswith(blob_prefix)
                    and path[len(blob_prefix) :] in image["blobs"]
                ):
                    identity = path[len(blob_prefix) :]
                    data, media, status = (
                        image["blobs"][identity],
                        "application/octet-stream",
                        200,
                    )
            self.send_response(status)
            for key, value in {
                "Docker-Distribution-API-Version": "registry/2.0",
                "Content-Type": media,
                "Content-Length": str(len(data)),
                "Connection": "close",
            }.items():
                self.send_header(key, value)
            if identity:
                self.send_header("Docker-Content-Digest", identity)
            self.end_headers()
            if not head:
                self.wfile.write(data)
            self.close_connection = True

        def do_GET(self):
            self.reply(False)

        def do_HEAD(self):
            self.reply(True)

    with HTTPServer(("127.0.0.1", 0), Handler) as server:
        authority = f"127.0.0.1:{server.server_port}"
        refs = {
            name.rsplit("/", 1)[1]: {
                "ref": f"{authority}/{name}:latest",
                "digest": image["digest"],
            }
            for name, image in records.items()
        }
        for name, image in refs.items():
            recipe = work / f"recipe-{name}"
            recipe.mkdir(mode=0o700)
            write(recipe / "Dockerfile", f"FROM {image['ref']}\n".encode())
        for name in ["context", "docker"]:
            (work / name).mkdir(mode=0o700)
        json_write(
            work / "expected.json",
            {"component_fixture_only": True, "registry": authority, "images": refs},
        )
        rules = [
            {
                "action": "DENY",
                "selector": {"identifier": prefix + "://*", "match_type": "WILDCARD"},
            }
            for prefix in ["docker-image", "http", "https", "git"]
        ]
        rules.append(
            {
                "action": "ALLOW",
                "selector": {
                    "identifier": f"docker-image://{authority}/fixture/*",
                    "match_type": "WILDCARD",
                },
            }
        )
        json_write(work / "source-policy.json", {"version": 1, "rules": rules})
        write(
            work / "buildkitd.toml",
            f'[registry."{authority}"]\n  http = true\n'.encode(),
        )
        print(
            json.dumps({"ready": True, "registry": authority, "work": str(work)}),
            flush=True,
        )

        def stopped(_number, _frame):
            raise KeyboardInterrupt

        signal.signal(signal.SIGTERM, stopped)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass
        finally:
            request_log.close()


def load(path, native=False):
    with path.open("rb") as source:
        data = source.read(LIMIT + 1)
    require(len(data) <= LIMIT, "conversion output exceeds 1 MiB")
    if native:
        require(
            data.startswith(b"result.json\n"), "missing native result.json response"
        )
        data = data.removeprefix(b"result.json\n")
    return json.loads(data)


def sources(converted):
    require(isinstance(converted.get("def"), dict), "missing conversion definition")
    return sorted(
        {
            operation["source"]["identifier"]
            for operation in converted["def"].values()
            if "source" in operation
            and operation["source"]["identifier"].startswith("docker-image://")
        }
    )


def compare(args):
    expected = load(args.work / "expected.json")
    observations = {}
    for name in ["zero", "layer"]:
        native = load(getattr(args, f"{name}_native"), native=True)
        helper = load(getattr(args, f"{name}_helper"))
        native_sources = sources(native)
        require(
            native_sources == sources(helper["convert_llb"]),
            f"{name}: conversion image sources disagree",
        )
        image = expected["images"][name]
        matched = [
            item
            for item in helper["resolutions"]
            if item["kind"] == "docker-image"
            and item["requested"].startswith("docker-image://" + image["ref"])
            and item["digest"] == image["digest"]
        ]
        require(matched, f"{name}: helper omitted fixture manifest identity")
        require(
            not native_sources
            if name == "zero"
            else any(image["digest"] in source for source in native_sources),
            f"{name}: native source-set control differs",
        )
        observations[name] = {
            "expected_manifest": image["digest"],
            "native_sources": native_sources,
            "resolutions": matched,
        }
    json_write(
        args.work / "comparison.json",
        {
            "component_fixture_only": True,
            "normal_cli_consumer": False,
            "production_jail": False,
            "common_claims": False,
            "production_proxy": False,
            "runtime_execution_boundary_proven": False,
            "observations": observations,
        },
    )
    print(json.dumps({"compared": True, "receipt": str(args.work / "comparison.json")}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    server = modes.add_parser("serve")
    server.add_argument("--work", type=Path, required=True)
    comparison = modes.add_parser("compare")
    comparison.add_argument("--work", type=Path, required=True)
    for name in ["zero-native", "zero-helper", "layer-native", "layer-helper"]:
        comparison.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    args.work = args.work.resolve()
    serve(args.work) if args.mode == "serve" else compare(args)


if __name__ == "__main__":
    main()
