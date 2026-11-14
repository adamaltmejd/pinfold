# Headless Chromium on macOS

Date: 2026-11-14. Ticket #298, project-box plan probe 4. Repository
`2a9bf28`, branch `probe/mac-chrome`. Apple container 1.7.1, arm64.
This is the original probe's record, condensed without rerunning a box.

## Result

Chromium 156.0.8078.79 starts and renders JavaScript with its sandbox off.
The observed processes keep the host's numeric UID/GID, empty capability
sets and `NoNewPrivs: 1`. The root is read-only and only loopback exists.
GitHub answers through pinfold's proxy. It refuses example.com and the
observed background HTTP requests. Guest-only CDP supports navigation,
evaluation, screenshots and PDF output.

Chromium also attempted five external UDP connects outside the proxy.
All failed with `ENETUNREACH`. No successful bypass was observed. This is
a bounded observation, not proof about future browser tasks.

## Evidence and reproduction

The adjacent [directory](2026-11-14-headless-chrome-probe/) keeps three
recipes, five probe scripts, the HTML fixture and [output.txt](2026-11-14-headless-chrome-probe/output.txt).
The output names its original source files. It distinguishes raw excerpts,
selected fields and summaries of the full logs reviewed before deletion.
Full logs, duplicate copies and binary files are no longer retained.
The scripts and recipes are unchanged. No box was rerun for this edit.

The original entry points, from the clone's root, were:

```sh
RUSTC_WRAPPER= cargo build --locked
python3 docs/archive/2026-11-14-headless-chrome-probe/build.py
python3 docs/archive/2026-11-14-headless-chrome-probe/run.py
python3 docs/archive/2026-11-14-headless-chrome-probe/run.py explicit-proxy
python3 docs/archive/2026-11-14-headless-chrome-probe/minimal.py
python3 docs/archive/2026-11-14-headless-chrome-probe/finish.py
```

The second `run.py` invocation recovered artifacts after the export error
below. The scripts need the real container service. They generate the box
specs and outputs; those generated files are omitted here. Removed
`pack.py` and `analyze.py` only compressed or summarized logs.

`build.py` creates isolated XDG config/state under a short `/private/tmp/pfc.*`
root, keeps the real cache, disables update checks and stamps maintenance
before its first pinfold command. `run.py` holds `box up` stdin open and
waits for `ready`. Home and work are separate 1 GiB disks; memory is 2 GiB.
`minimal.py` uses the plain Chromium image. `finish.py` checks boxes/disks
are absent before removing scratch state. No host login, credential file,
harness or injecting route was used. The allowlist was only `api.github.com`.

## Build cost and hosts

`probe-chrome-base` copied the default profile recipe. `probe-chrome`
adds Debian Chromium with `--no-install-recommends`. The separate
`probe-chrome-tools` adds strace and Python. Recipes strip setuid/setgid bits.

| Profile | Build seconds | Runtime arm64 variant bytes |
|---|---:|---:|
| probe-chrome-base | 9.518 | 71,037,562 |
| probe-chrome | 40.477 | 348,154,131 |
| probe-chrome-tools | 22.404 | 360,315,141 |

Chromium adds 264.28 MiB; tools add 11.60 MiB. These are runtime variant
sizes, not unpacked sizes or physical allocation. Apt reported 194 MB
downloads and 752 MB additional installed space. `/usr/lib/chromium` was
379M; accessible root files were 906M, with permission denials.
These single builds shared a cache; the base apt layer was already cached.
Free space before the builds was 14, 13 and 25 GiB, above the 12 GiB floor.

The full apt HTTP diagnostics named only `deb.debian.org`, with no redirect
host. Installing Chromium needed no Google download host. This inventories
apt requests, not all build traffic. Apple builds have unrestricted egress;
no shared-builder packet capture or Linux build run was made.

## Startup, controls and navigation

The plain image's `chromium --headless --no-sandbox
--user-data-dir=/home/user/minimal --dump-dom https://api.github.com/`
returned GitHub's JSON and exited 0. Debian's package used pinfold's proxy
environment without an explicit flag. The traced case also succeeded with
`--proxy-server=http://127.0.0.1:3128`.

Without `--no-sandbox`, `--headless --dump-dom about:blank` exited 1 with
`No usable sandbox!`. The optional chromium-sandbox package was absent;
user namespaces were disabled. This does not test every sandbox arrangement.
Debian's launcher added `--disable-dev-shm-usage` with 64 MiB `/dev/shm`.
Its effective flags are summarized in output.txt. The probe added no
background-networking, QUIC or WebRTC policy disablement.

The `/proc` snapshot covered browser, zygotes, GPU, network, storage and
renderer processes: UID 501, GID 20, all capability sets zero, no new
privileges. Host numeric IDs matched. Root writes failed with errno 30;
`user.max_user_namespaces` was 0. The file scan printed no setuid/setgid
file, but its traversal was incomplete and exited 1. Profiles were on the
home disk. D-Bus and mDNS diagnostics did not prevent the demonstrated work.

In the same box CDP observed GitHub HTTP 200 at `127.0.0.1:3128`, HTTPS
example.com `net::ERR_TUNNEL_CONNECTION_FAILED`, and HTTP example.com
`HTTP ERROR 403` / `net::ERR_HTTP_RESPONSE_CODE_FAILURE`. The retained proxy
entries name `allowlisted` or `not allowlisted`. The earlier environment-only
case also returned GitHub HTTP 200.

## Background traffic and bypass observation

The final fresh-profile run lasted about 73 seconds: navigation/rendering,
12 seconds of WebRTC gathering and another 60 seconds of observation.
strace followed descendants with `-f -e trace=network`; NetLog used
capture mode Everything. Proxy logs alone cannot show bypass attempts.

All 15 final egress entries remain. They show GitHub allowed twice;
example.com refused five times; clients2.google.com and redirector.gvt1.com
once each; update.googleapis.com four times; optimizationguide-pa.googleapis.com
once. One empty-host refusal was `header timeout`; its initiating browser
task was not established. Counts include retries and repeated tunnels.
The earlier environment-only and plain runs added no host. NetLog paths
identify time, dictionary, component-update and model requests. The
observed background HTTP requests reached the proxy and were refused.

The retained external syscall lines include three UDP connects to
`[2001:4860:4860::8888]:443`, one to `8.8.8.8:53` and one to
`[2001:4860:4860::8888]:53`, including its resumed failure. All failed.
The IPv6 port-443 probes correlate with NetLog IPv6 reachability checks;
port-53 calls occurred during WebRTC work. No DNS payload followed them.
These were connectivity attempts, not successful DNS, DoH, QUIC or STUN.
AF_UNIX IPC, AF_NETLINK discovery and loopback traffic were separate.
The full-trace audit counted 15 loopback proxy connects.

NetLog had no DoH servers and disabled secure DNS transactions. The full-log
audit found no DoH or distinct Safe Browsing request and only QUIC pool
bookkeeping/shutdown, with no session or handshake. This does not show
Safe Browsing is disabled or predict later traffic.

The fixture created a data channel with `stun:example.com:3478`. After
12 seconds gathering remained pending, with no candidates or error events.
The full trace had no port-3478 traffic. Why gathering remained pending
was not established. This does not establish WebRTC proxy behavior on a
network permitting an exchange. The retained result omits transient SDP.

## Rendering and agent control

The fixture changed its text to `JavaScript rendered: 42` and drew three
colored canvas blocks. The plain image's screenshot command exited 0.
CDP `Runtime.evaluate`, `Page.captureScreenshot` and `Page.printToPDF`
produced artifacts on the work disk, exported after teardown.
The original visual inspection found the rendered text and blocks.
Removed binary files showed:

| File | Bytes | Content |
|---|---:|---|
| minimal.png | 19,035 | Plain-image rendering of the fixture |
| work/rendered.png | 19,176 | CDP rendering of the fixture |
| work/rendered.pdf | 21,077 | One-page fixture PDF; extracted text included 42 |
| pdf-preview.png | 24,455 | Raster preview of that PDF |

The three compressed syscall traces were also removed; the relevant final
calls and full-log audit summaries remain in output.txt.

`guest.py` starts Chromium with loopback debugging on port 9222, waits for
`DevTools listening on ...`, reads `/json/version`, creates a page with
`PUT /json/new?about:blank` and uses its WebSocket URL. Its stdlib driver
bypasses the proxy for loopback CDP. A host agent can invoke it through
`pinfold box exec`; no host debugging port is exposed. Pinfold HTTP routes
drop WebSocket Upgrade, so a proxy route is not a CDP transport.

## Failures and limits

The default build failed before compilation because sccache was denied.
Clearing `RUSTC_WRAPPER` built the binary in 22.13 seconds. Runtime access
needed execution outside the tool sandbox. The first driver's reason-phrase
check rejected Chromium's valid HTTP 101 `WebSocket Protocol Handshake`;
both attempts stopped before navigation. It now checks status and accept.

The next run completed browser cases but export failed with
`destination-exists`. The script then removed its disks, losing that run's
artifacts. The final run used a new destination and exported successfully.
The retained script keeps disks if export fails. These were probe errors.

Owner decision: this supports an optional Chromium image on macOS, not a
built-in profile or background-service policy. No allowlist expansion is
proposed. Chromium's sandbox stays off as project-box plan decision 5
requests; renderer isolation is not established.

Not established: Linux, Google Chrome/Playwright, long-running updates,
a distinct Safe Browsing request, forced DoH/QUIC, working WebRTC/STUN,
a complete build traffic inventory, large pages or memory pressure.
No implementation, default profile, security control or test changed.

## Retained resources and checks

Original cleanup found no probe boxes or disks and removed scratch state.
The running container service was left running. The three probe images
were left with `latest` and a unique tag: base `18e7fcd48b493cb8-90632`,
Chromium `18e7fcd68a33cee0-90693`, tools `18e7fcdff9315390-90978`.
Their full names and IDs remain in output.txt. These are historical
observations; this edit did not query or alter runtime resources.

The original fmt and clippy checks passed. No test was added or run.
Retained text was checked for real usernames and local-network host names
or addresses. Home paths use `/Users/user` or `/home/user`; IP literals
are loopback or the public DNS probe destinations above. URL queries,
WebRTC SDP and bundled vendor-key material are omitted from excerpts.
Nothing was pushed and no PR was opened.
