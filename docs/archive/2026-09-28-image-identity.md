# Image identity and name retirement, 2026-09-28

Issues #63, #64 and #65, from Switchyard on yard-sthlm (Debian 13 in a
Proxmox LXC, podman 5.4.2, pinfold 0.0.6).

## Per-build ids (#63, reopening #52)

#52 was declined on the premise that identical builds share every layer
and cost nothing on disk, with a disk measurement as the re-admission
condition. #63 brings it. On rootless podman whose storage cannot shift
ids (native overlay, the usual unprivileged LXC), `--userns=keep-id`
makes containers/storage copy an image's layers, uid-mapped, the first
time a box starts from an image id. The copy is recorded per image, not
per layer. For a label-only caller build on the default profile:

| step | disk | time |
|---|---|---|
| `image build`, fully cached | +0 MB | 3.3 s |
| first `box up` from the new id | +326 MB | 3.0 s idle, 14–19 s under load |
| second `box up`, same id | +0 | 0.4 s |

The copy holds the storage lock; `box list` waited up to 13 s behind it.
Two days of Switchyard's e2e suite left 377 images and 72 GB.

Decision: a build's identity moves from the `dev.pinfold.build` label to
its unique tag, so a caller build with unchanged layers and labels
returns the existing image, and retention counts builds by tag.
keep-id stays: it is guarantee 10's mechanism.

## Base pull on a local base (#64)

The base-digest pull for `FROM pinfold/profile-default:latest` retries
three times against `localhost` and costs 3.0 s of a 3.3 s cached build.
A base pinfold built has no registry; its digest is read locally.

## Retiring a name (#65, re-filing half of #37)

#37's condition was met: a caller retires names (a project per e2e
scenario) and the leftovers grew (377 images). `pinfold clean --unused`
was not chosen: `--unused 0s` also removes idle projects' state, so a
caller retiring its own names would sweep everyone's. `image rm NAME`
removes exactly one name's unused images.
