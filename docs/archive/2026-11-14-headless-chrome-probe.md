# Headless Chromium on macOS

Date: 2026-11-14. Ticket #298, project-box plan probe 4. Repository
`2a9bf28`, branch `probe/mac-chrome`. Apple container 1.7.1, arm64.
The binary was built in this clone with `cargo build --locked`.

## Result

Chromium 156.0.8078.79 starts and renders JavaScript with its own sandbox
off. The box keeps the host UID, empty capabilities, `no_new_privs`, a
read-only root and only loopback. GitHub answers through pinfold's proxy.
The proxy refuses example.com and the observed background HTTP requests.
A guest-only remote debugging port works for navigation, evaluation,
screenshots and PDF output.

Chromium does not send every network operation through the HTTP proxy.
It also makes direct UDP connectivity probes. All five observed external
UDP connects failed with `ENETUNREACH`. No successful bypass was observed.
This is a bounded observation, not proof about every future browser task.

## Setup and reproduction

The adjacent [record directory](2026-11-14-headless-chrome-probe/) contains
the scripts, recipes, box specs and output. Commands in the logs use
`/Users/user`; `$W` is the probe's private, short `/private/tmp/pfc.*` root.
No host login, credential file, harness or injecting route was used.
The box allowlist was exactly `api.github.com`.

The entry points, from the clone's root, were:

```sh
RUSTC_WRAPPER= cargo build --locked
python3 docs/archive/2026-11-14-headless-chrome-probe/build.py
python3 docs/archive/2026-11-14-headless-chrome-probe/run.py
python3 docs/archive/2026-11-14-headless-chrome-probe/run.py explicit-proxy
python3 docs/archive/2026-11-14-headless-chrome-probe/minimal.py
python3 docs/archive/2026-11-14-headless-chrome-probe/pack.py
python3 docs/archive/2026-11-14-headless-chrome-probe/analyze.py
python3 docs/archive/2026-11-14-headless-chrome-probe/finish.py
```

The second `run.py` invocation repeats the working browser case to recover
artifacts after the export-script error described below. Reproduction
needs the real container service. These scripts build only probe-owned
profiles. `build.py` creates isolated XDG config and state, leaves the real
cache selected, disables update checks and stamps maintenance before its
first pinfold command. `run.py` holds `box up` stdin open and waits for
`ready`. Work and home are separate 1 GiB disks, with 2 GiB box memory.
`minimal.py` uses the Chromium image without the observation tools.
`finish.py` removes the scratch root and its temporary `scratch-path`
pointer only after boxes and disks are absent.

## Build cost and hosts

`pinfold profile new probe-chrome-base` copied the built-in default
recipe. `pinfold build --profile probe-chrome-base` built it under a new
name. `probe-chrome` extends that local image with Debian's Chromium,
using `apt-get install --no-install-recommends chromium`. A separate
`probe-chrome-tools` image adds strace and Python for observation and CDP.
Each recipe strips setuid/setgid bits after installation.

| Profile | Build seconds | Runtime-reported arm64 variant bytes | MiB |
|---|---:|---:|---:|
| probe-chrome-base | 9.518 | 71,037,562 | 67.75 |
| probe-chrome | 40.477 | 348,154,131 | 332.03 |
| probe-chrome-tools | 22.404 | 360,315,141 | 343.62 |

Chromium adds 277,116,569 reported image bytes, or 264.28 MiB.
The tools add another 12,161,010 bytes, or 11.60 MiB. These are the
runtime's variant sizes, not the image's unpacked filesystem size or
physical allocation on this Mac. Apt reported 194 MB of package downloads
and 752 MB of additional installed space for Chromium and its dependencies.
`du -sh /usr/lib/chromium` reported 379M; the plain image's accessible
root files occupied 906M. The latter measurement had permission denials.

These are single builds with a shared cache. The base's apt layer was
already cached, as its image history shows. They are not cold-build
benchmarks. Free disk before the three image builds was 14, 13 and 25 GiB;
all passed the required 12 GiB floor. No other builder's images were
modified or pruned.

The recipes save apt HTTP acquisition diagnostics inside the image.
`probe-chrome-apt.txt` and `probe-chrome-tools-apt.txt` contain only
`deb.debian.org` as HTTP Host, with no redirect host. Chromium therefore
needed no additional application download host beyond the Debian mirror
already used by the default recipe. No Google download host was needed
to install the browser. This inventories apt requests, not all build
traffic. Apple's build networking is unrestricted; no packet capture of
the shared builder, registry refresh or registry authentication traffic
was taken. Linux build egress was not run.

## Startup flags and controls

The plain image succeeded with this command and exited 0:

```sh
pinfold box exec probe-chrome-minimal -- chromium \
  --headless --no-sandbox --user-data-dir=/home/user/minimal \
  --dump-dom https://api.github.com/
```

Its output contained GitHub's JSON document. No explicit proxy flag was
needed by this Debian package: pinfold's supplied proxy environment was
enough. The traced case also succeeded with
`--proxy-server=http://127.0.0.1:3128`. That flag makes the intended proxy
explicit but was not a prerequisite observed here.

Without `--no-sandbox`, `--headless --dump-dom about:blank` exited 1:

```text
No usable sandbox!
```

This package did not include the optional chromium-sandbox package, and
the box disables user namespaces. This case establishes the failure of
the installed package, not every possible Chromium sandbox arrangement.

The launcher automatically added `--disable-dev-shm-usage` because
`/dev/shm` was 64 MiB. Its shipped `/etc/chromium.d/dev-shm` applies that
flag below 3.8 GB. It also supplied `--disable-pings`, `--media-router=0`,
`--no-default-browser-check`, `--enable-gpu-rasterization`,
`--show-component-extension-options`, `--enable-remote-extensions` and an
empty `--load-extension`. The recorded process command lines show the
effective flags. This is stock Debian launcher behavior, not an upstream
Chrome default. No additional `--disable-background-networking`,
`--disable-quic` or WebRTC policy was applied by the probe.

`explicit-proxy.jsonl.txt` records `/proc` observations from the browser,
zygotes, GPU, network, storage and renderer processes: UID 501, GID 20,
all capability sets zero and `NoNewPrivs: 1`. It also records:

```text
interfaces: ["lo"]
/dev/vdb / ext4 ro,relatime 0 0
user.max_user_namespaces: 0
[Errno 30] Read-only file system: '/probe-chrome-write'
```

The observed UID/GID matched `id` in `guest-facts.txt` and the host's
numeric IDs in `checks.txt`. Writable browser
profiles lived only on the home disk. The scan printed no setuid/setgid
file. The broader facts command exited 1 because its filesystem traversal
was not wholly readable; it is not a complete privileged filesystem audit.
Missing D-Bus services and a failed mDNS responder produced diagnostics,
but did not prevent navigation or rendering.

## Allowed and denied navigation

The CDP driver navigated to GitHub, then HTTPS and HTTP example.com, in
the same box. It recorded GitHub HTTP 200 with remote address
`127.0.0.1:3128`. HTTPS example.com returned
`net::ERR_TUNNEL_CONNECTION_FAILED`; HTTP example.com displayed
`HTTP ERROR 403` and CDP returned `net::ERR_HTTP_RESPONSE_CODE_FAILURE`.
These matched the proxy decisions in `egress.jsonl`:

```json
{"decision":"allowed","host":"api.github.com","reason":"allowlisted","time":"2026-11-14T18:33:40Z"}
{"decision":"refused","host":"example.com","reason":"not allowlisted","time":"2026-11-14T18:33:40Z"}
```

The earlier environment-only case also recorded GitHub HTTP 200 and
the same allowlisted decision. Its stdout is `env-only.jsonl.txt` and
its separate log snapshot is `env-only-egress.jsonl`.

## Automatic traffic and bypass observation

The final traced case started with a fresh browser profile and background
networking enabled. It ran for about 73 seconds: navigation and rendering,
a 12-second WebRTC case, then a further 60-second observation interval.
strace followed all descendants with `-f -e trace=network`; Chrome also
wrote NetLog with capture mode Everything. The egress log alone cannot
show attempts that bypass the proxy.

Every host in the final egress log is below. The earlier environment-only
and plain-image runs added no other host.

| Host | Final decisions | Observed URL or purpose |
|---|---|---|
| api.github.com | 2 allowed, `allowlisted` | Explicit navigation |
| example.com | 5 refused, `not allowlisted` | Explicit denied navigation |
| clients2.google.com | 1 refused, `not allowlisted` | `/time/1/current` |
| redirector.gvt1.com | 1 refused, `not allowlisted` | `/edgedl/chrome/dict/en-us-10-2.bdic` |
| update.googleapis.com | 4 refused, `not allowlisted` | HTTP/HTTPS POST `/service/update2/json` |
| optimizationguide-pa.googleapis.com | 1 refused, `not allowlisted` | POST `/v1:GetModels` |

One additional decision had an empty host and reason `header timeout`.
It names no destination; its precise initiating browser task was not
established. Denial counts include browser retries and repeated tunnels.
They are not counts of distinct pages. Complete lines are retained.

All observed background HTTP requests reached the proxy and were refused.
NetLog identifies component update requests and later retries. It does
not show a distinct Safe Browsing service request. That does not establish
that Safe Browsing is disabled, or how its later requests would behave.

strace also records these non-loopback UDP connects:

| Destination | Count | Result |
|---|---:|---|
| `[2001:4860:4860::8888]:443` | 3 | `ENETUNREACH` |
| `8.8.8.8:53` | 1 | `ENETUNREACH` |
| `[2001:4860:4860::8888]:53` | 1 | `ENETUNREACH` |

The three IPv6 port-443 calls correlate with NetLog's
`HOST_RESOLVER_MANAGER_IPV6_REACHABILITY_CHECK`. The port-53 calls occur
during the WebRTC work. No DNS payload followed them. They are direct
connectivity probes, not evidence of successful DNS queries, DoH, QUIC or
STUN traffic. They bypass the HTTP proxy as attempted operations, but
the guest has no route for them. No direct external connection succeeded.
AF_UNIX IPC, AF_NETLINK interface discovery and guest loopback CDP/proxy
connections were considered separately. `analysis.json` lists the
external call sites and the 15 loopback proxy connects.

NetLog's DNS configuration had no DoH servers and
`can_use_secure_dns_transactions: false`. No DoH request was observed.
QUIC events were pool bookkeeping and shutdown, with no QUIC session or
handshake. The UDP port-443 probes above are not QUIC sessions.

The fixture explicitly created an `RTCPeerConnection` and data channel
with `stun:example.com:3478`. After 12 seconds it was still gathering,
with no ICE candidates or error events. strace showed no port-3478
traffic. With only loopback, the case produced no ICE candidates or STUN
traffic; why gathering remained pending was not established. It does not
establish WebRTC's proxy behavior on a network
that permits such an exchange. No successful bypass was seen here.

## Rendering and agent control

The project disk held `fixture.html`, whose JavaScript changed the text
to `JavaScript rendered: 42` and drew three colored canvas blocks.
The plain image wrote `/workspace/minimal.png` with Chromium's
`--screenshot` flag and exited 0. It was exported after box teardown
and is retained as `minimal.png`.

The traced run used CDP `Runtime.evaluate`, `Page.captureScreenshot` and
`Page.printToPDF`. It wrote `work/rendered.png` and `work/rendered.pdf`
on the project disk. `disk export` copied them to a new host destination
after `box down`. Both images and the rendered PDF were inspected
visually. `pdftotext` found the rendered value 42; `pdfinfo` reported a
one-page, 21,077-byte PDF. The extracted text and preview are retained.

An agent in the box can start:

```sh
chromium --headless --no-sandbox --user-data-dir=/home/user/chrome \
  --proxy-server=http://127.0.0.1:3128 \
  --remote-debugging-address=127.0.0.1 --remote-debugging-port=9222 \
  about:blank
```

It waits for Chromium's `DevTools listening on ...` readiness line,
reads `http://127.0.0.1:9222/json/version`, creates a page with
`PUT /json/new?about:blank`, then connects to its returned WebSocket URL.
`guest.py` demonstrates navigation, JS evaluation and export with only
Python's standard library. A host agent can invoke that driver with
`pinfold box exec`; no debugging port is exposed on the host. Direct guest
HTTP clients must bypass their proxy for this loopback service, as the
driver's explicit empty ProxyHandler does. Pinfold's HTTP routes drop
WebSocket Upgrade, so a proxy route is not a CDP transport.

## Failures, limits and owner decisions

The default host `cargo build --locked` failed before compilation because
sccache was not permitted by the sandbox. Clearing `RUSTC_WRAPPER` built
the real binary in 22.13 seconds. Runtime access also needed execution
outside the tool sandbox. These were host execution constraints.

The first driver incorrectly required the HTTP 101 reason phrase
`Switching Protocols`; Chromium used `WebSocket Protocol Handshake`.
Both first attempts stopped before navigation. The driver now checks
status 101 and Sec-WebSocket-Accept. Their output and traces remain in
`failed-driver-handshake/`.

The next run completed its browser cases but reused the first export
destination. Pinfold correctly refused it as `destination-exists`.
The probe script then removed its disks, losing that run's new artifacts.
Its stdout and egress log remain in `completed-cdp-export-failed/`.
The final run used a new destination and retained its artifacts. The
script now refuses to remove disks if export fails. These are probe-script
errors, not pinfold failures.

Owner decision: this establishes a workable optional Chromium image on
macOS. It does not decide whether to add it to a built-in profile or which
background services should be disabled for product use. None of the
denied Google hosts is needed by the demonstrated workflow. No allowlist
expansion is proposed. Chrome's sandbox remains off as decision 5 of the
project-box plan requests; the probe does not establish renderer isolation.

Not established: Linux; Google Chrome or Playwright; long-running or
scheduled update behavior; a distinct Safe Browsing request; forced DoH
or QUIC traffic; a working WebRTC/STUN exchange; a complete packet-level
build host inventory; or behavior under large pages and memory pressure.
No implementation, default profile, security control or test was changed.

## Retained resources and checks

The only repository changes are this dated record and its scripts,
recipes, specs, raw output and rendered artifacts. Host usernames were
normalized. A browser-bundled vendor key in a locally denied optimization
URL was redacted; no user credential was supplied. Raw syscall traces are
gzip-compressed losslessly. `pack.py` verifies decompression before
removing their text copies.

All probe boxes and all four named disks were removed. The scratch config,
state and exports were removed. The container service was already running
and remains running. Other builders' resources and buildkit were left
alone. `cleanup.txt` records the final empty probe-box and disk inventories.

The three images remain, each with `latest` and its unique build tag:

| Repository | Unique tag |
|---|---|
| localhost/pinfold/profile-probe-chrome-base | 18e7fcd48b493cb8-90632 |
| localhost/pinfold/profile-probe-chrome | 18e7fcd68a33cee0-90693 |
| localhost/pinfold/profile-probe-chrome-tools | 18e7fcdff9315390-90978 |

Their IDs and full inventory are in `images-left.json`.
`cargo fmt --check` passed. `RUSTC_WRAPPER= cargo clippy --all-targets
--locked -- -D warnings` passed. No test was added or run; the suite is
suspended. The record is committed on the working branch. Nothing was
pushed and no PR was opened.
