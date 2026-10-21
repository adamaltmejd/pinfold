# Per-start mount image cost on Linux (2026-10-21)

Since #141, a Linux box copies every mount into a read-only ext4 image at each start with `mkfs.ext4 -d`. That covers the caller's mounts, pinfold's harness and `share/`. These measurements show what the copy adds to the time `box up` takes to reach `ready`.

## Setup

- Host: the homelab trial VM. Debian 13, kernel 6.12, 2 vCPUs, 4 GB RAM, nested KVM, disk-backed `/tmp`.
- Build: a debug build of the PR #157 tree.
- Clock: wall time from writing the spec to `box up`'s first stdout line.
- Image: `pinfold/profile-default:latest`. The root disk was already cached, so its one-time export is not in these numbers.
- The page cache was warm after the first run.

## Results

| Spec | Mount size | Time to `ready` | Added by mount images |
|---|---|---|---|
| no mounts | none | 2.65–2.73 s (one cold outlier at 6.3 s) | none |
| one mount | 1.7 MB, 142 entries | 2.82–2.92 s | about 0.15 s |
| one mount | 53 MB, 3,752 entries | 3.04–3.10 s | about 0.3 s |
| `harness: "pi"` | 110 MB, 239 entries | 3.98–4.15 s | about 1.3 s |

Each row is three runs.

## Reading

- The cost follows bytes copied more than entry count.
- pi's harness is the largest tree a typical box mounts, and it adds about 1.3 s to every pi start.
- A caller's source tree of a few tens of MB adds well under a second.
- On CI the full e2e suite ran in 290.7 s, against a 300 s budget.

## Revisit

Revisit if pi start latency matters. Making the harness image once per harness version, rather than per start, would remove most of the cost.
