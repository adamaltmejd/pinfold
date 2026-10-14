# Issue 89: cold keep-id mapping blocks native inventory

The bounded-listing fix is committed as `70c1832`. A separate real Podman
probe reproduced a storage mechanism that can exceed the original
30-second caller deadline. It identifies the holder in this probe, not
the holders of issue 89's two original LXC failures. Issue 89 remains open.

## Environment and method

The leased Debian 13 VM ran rootless Podman 5.4.2 as UID/GID 1000,
kernel 6.12.107+deb13-cloud-amd64, ext4, native overlay and systemd
cgroup v2. Storage reported `Supports shifting: false` and
`Using metacopy: false`. No FUSE, user-namespace change, runtime stub,
artificial held lock or strace was used in these probes.

An imported image contained one layer of empty, root-owned files. A real
`podman create --userns=keep-id --network=none --entrypoint=/unused`
prepared it. The entrypoint was never executed. An outside observer
matched actual lock device/inode identities in `/proc/locks` and recorded
process roles, helper ancestry and thread wait channels/syscalls. It
started native `podman ps --all --format json` after the creator held the
image-store write lock continuously for 0.2 seconds. In the larger probe,
it also started the committed Pinfold binary's labeled listing with fresh
XDG roots. Each create had a 180-second external observer limit.

The same image, store and mapping were then used for a second create while
the first stopped container remained. No deadlines, runtime options or
storage configuration changed between cold and warm creation. The warm
native listing started after creation, since no long-held lock was seen;
its timing is an idle comparison, not a concurrent warm-start measurement.

## Results

| Fixture | Cold create | Concurrent native list | Warm phase upper bound | Idle native list |
| --- | ---: | ---: | ---: | ---: |
| 250,000 empty files | 15.732 s | 15.458 s | 0.083 s | 0.064 s |
| 750,000 empty files | 43.913 s | 43.690 s | 0.289 s | 0.127 s |

All creates and native listings exited zero. Import time was outside these
measurements: 21.598 and 65.488 seconds respectively. These are observed
single runs, not benchmark distributions. Completion times were sampled;
the observer records a timestamp before reading process metadata. The
750,000-file warm phase's 0.289 seconds includes its later idle listing,
so it is an upper bound on create time. Empty files exercise metadata
walking and copy-ups; they do not measure copying populated file data.

In the larger probe:

- Creator PID 11076 held `overlay-images/images.lock`, identity
  `08:01:262159`, and `overlay-layers/layers.lock`, identity `08:01:262166`.
- Its child PID 11085 had the exact re-exec role `storage-chown-by-maps`,
  parent 11076 and start tick 55509. A thread was observed in syscall 260
  (`fchownat` on x86-64) with wait channel `ovl_copy_up_one`, including at
  3.847 and 19.754 seconds. This directly connects ownership adjustment
  and overlay copy-up to the lock holder.
- Native listing PID 11091 and Pinfold's runtime client PID 11093 were
  write-lock waiters on that same image-store inode. The latter was
  observed in `fcntl_setlk`.
- Pinfold PID 11092 exited 1 after 20.038 seconds with `runtime-timeout`
  and empty stdout. At 20.309 seconds, runtime client 11093 was no longer
  a waiter. Native waiter 11091 remained, and creator 11076 kept working.
- The creator completed successfully at 43.913 seconds. Pinfold did not
  kill the writer to meet its listing deadline.
- Image metadata after cold creation recorded mapped layer
  `29ca7bf04bdb7b3637834cb2bea7bf9eea343ba67a0f8e20a31788f3cc6d55eb`.
  The warm create retained that mapping and had no observed chown helper.
  The metadata capture does not independently establish a pre-create
  absence of the mapping.

After removing both stopped containers, another identical keep-id create
succeeded in 0.152 seconds. Removing the warm-up container therefore did
not discard the useful mapping in this store. Pinfold's labeled listing
then returned exit zero with empty stdout; these synthetic containers had
no Pinfold owner labels. Preservation of a live Pinfold box and its owner
under a stalled listing was separately verified by guarantee 9's real-lock
test, recorded in `2026-10-12-test-audit.md`.

## Ownership and next action

The observed native stall belongs to Podman's containers/storage mapping
preparation. It matches the open upstream
[Podman issue 16541](https://github.com/podman-container-tools/podman/issues/16541)
and its [maintainer explanation](https://github.com/podman-container-tools/podman/issues/16541#issuecomment-1352790422).
Podman's [performance guide](https://github.com/podman-container-tools/podman/blob/main/docs/tutorials/performance.md#choosing-a-storage-driver)
documents this cold native rootless mapping cost. The related
[PR 28393](https://github.com/podman-container-tools/podman/pull/28393)
was still open when checked; it carries mappings into image pull. It is
not a verified fix for existing built images or this original workload.

Preparing the exact image and keep-id mapping once before concurrent work
is a supported native-overlay experiment. A create followed by removal is
sufficient in this VM. It preserves keep-id and needs no FUSE. A new image
or mapping can incur the cold cost again; cache pruning can remove it.
This is an operator preparation step, not a Pinfold startup guarantee.

Before attributing the original LXC failures to this mechanism, run the
unchanged workload there with actual lock identities, holder/helper
ancestry and mapping state recorded. Compare its cold and pre-mapped
images with the same storage configuration and concurrency. No further
production change is justified by this probe alone.

## Reproduction and retained evidence

`2026-10-12-native-overlay-lock-probe.py` is the diagnostic used. Run it
only in a disposable rootless native store with a working user bus. It
creates and removes two named containers and imports a synthetic image.
The image and cached mapping remain until the disposable store is erased.
Use `FIXTURE_FILES=750000 python3 <script>` for the larger probe. Its
default is the initial 250,000-file probe. An optional executable at
`/tmp/pinfold-candidate` adds the concurrent bounded-listing observation.
The 20 ms sampling pauses are in the outside diagnostic, not the e2e
suite or production binary.

Raw JSON observations were copied out to
`/private/tmp/pinfold-native-mapping-evidence.jsonl` and
`/private/tmp/pinfold-native-mapping-scaled.jsonl`. They contain synthetic
image/container IDs and process/lock metadata, with no credentials or
process environment. The VM was released after evidence collection.

The post-removal check was a separate command, not part of those JSONL
files. A Python observer timed `subprocess.run` of the same keep-id create
after the diagnostic removed both stopped containers. Its retained result
was:

```json
{"after_removal_create_seconds":0.15238191999992523,"exit":0,"stdout":"d094a81ee4e222a2e8b5fffb290f90b4bdc9bf6914333dfc2b716a211f1a6c83\n","stderr":""}
{"recovered_listing_exit":0,"stdout":"","stderr":""}
```
