"""Print a box spec. argv: NAME, then any of
  mount=HOSTDIR:GUEST   a read-only mount
  env=NAME=VALUE        a box-wide env entry
  disks=PREFIX          name the disks PREFIX-home and PREFIX-work
Every box lists the three harnesses only so their binaries are mounted; no
exec passes --harness, so no per-exec env is set. Its env holds HOME and the
harnesses' pinned defaults, box-wide. Routes are injecting routes to the
host fakes, one port each, with the header set of today's login route; no
login route is configured, so no host Codex login is asked. HOME and /work
are fresh disks named after the box."""

import json
import sys

name, args = sys.argv[1], sys.argv[2:]
prefix = next((a.split("=", 1)[1] for a in args if a.startswith("disks=")), name)
args = [a for a in args if not a.startswith("disks=")]
spec = {
    "name": name,
    "image": "localhost/pinfold/profile-default:latest",
    "harnesses": ["pi", "claude", "codex"],
    "mounts": [],
    "disks": [
        {"name": f"{prefix}-home", "guest": "/home/pi"},
        {"name": f"{prefix}-work", "guest": "/work"},
    ],
    "env": {
        "HOME": "/home/pi",
        "PI_TELEMETRY": "0",
        "PI_SKIP_VERSION_CHECK": "1",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
    },
    "egress": {
        "allow": [],
        "routes": {
            "claude": {
                "to": "http://127.0.0.1:18771",
                "headers": {"Authorization": {"from": "FAKE_CLAUDE_TOKEN", "prefix": "Bearer "}},
            },
            "anthropic": {
                "to": "http://127.0.0.1:18772",
                "headers": {"x-api-key": {"from": "FAKE_PI_KEY"}},
            },
            "codex": {
                "to": "http://127.0.0.1:18773",
                "headers": {
                    "Authorization": {"from": "FAKE_CODEX_TOKEN", "prefix": "Bearer "},
                    "ChatGPT-Account-ID": {"from": "FAKE_CODEX_ACCOUNT"},
                },
            },
            "raw": "127.0.0.1:18774",
        },
    },
    "cpus": 2,
    "memory": "4G",
}
for arg in args:
    kind, _, value = arg.partition("=")
    if kind == "mount":
        host, guest = value.split(":", 1)
        spec["mounts"].append({"host": host, "guest": guest})
    elif kind == "env":
        key, val = value.split("=", 1)
        spec["env"][key] = val
    else:
        sys.exit(f"unknown arg {arg}")
print(json.dumps(spec))
