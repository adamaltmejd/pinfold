# Managed build layer cache, 2026-10-29

This host CLI cut starts from
`a42d0d786593bc5f1450ab3ec8914aba8e4d6fbf`. Normal profile and project builds
now use the existing layer cache by default. `pinfold build --no-cache`, also
with `--profile NAME`, requests fresh RUN execution. Caller image-build defaults
and its existing --no-cache behavior are unchanged.

The operator observed real default-profile warm/repeat builds taking about
44.07 and 44.39 seconds on the prior head. Source inspection found the managed
Context::Alone path explicitly requesting cache=false. Those observations
motivate changing the normal cached-build behavior; they are not measurements
of this cut and do not establish a new latency result.

## Smallest source change and tradeoff

One cache bool on image::Build replaces Context::Dir's cache field and the
hardcoded cache=false for Context::Alone. Both contexts pass that bool to the
same BuildRequest. The normal build parser defaults it to true and recognizes
--no-cache; the caller-image parser continues forwarding its existing bool.
Apple and Linux already implement that request, so no backend, bootstrap,
artifact, helper, dependency or cache mechanism changes.

An unchanged recipe can reuse earlier RUN results, including installed package
versions. Normal builds no longer promise that package-update commands execute
every time. Use --no-cache to force those RUN steps again. Base refresh and
Linux's frozen native-discovery identities still run through their existing
paths. This flag does not introduce a different base-resolution policy or clear
mutable cache mounts.

Managed builds still receive an empty Containerfile-only context. Checksum ADD,
recipe fingerprints, selected-profile freshness hints, labels, unique refs and
retention are unchanged. The Always-applied controls, proxy and threat/trust
boundaries are unchanged. The actual private 1e9 bootstrap pin remains exact;
this host CLI change requires no new producer build. Its public URL remains
unpublished and private proof still uses the verified offline archive.

The binding Images contract, CLI usage table and README now state the new
default and flag. A future authorized CLI release must describe both caller
changes. This cut creates no release or tag.

## Existing coverage and proposed observation

No tests or assertion blocks change. The approved five critical build cases
and G44 keep their current scope. G23 covers caller A/B context, refs, IDs,
labels, Linux post-commit EIO, and ordinary failed F with bounded output and
unchanged refs. It does not directly cover normal profile/project cache reuse
or the new normal-build flag. Its coverage row therefore stays unchanged.
G15's single Linux project build and shared default-profile setup provide
changed-path success coverage, without a repeat/forced-refresh comparison.

The smallest parent-managed external observation is three normal CLI builds
of one disposable profile, using one unchanged recipe and a fixed existing
local base. A RUN writes a nonempty marker from guest randomness, for example
32 bytes rendered as hex in /cache-proof. Read the actual file through ordinary
box up/exec/down after each build:

1. Build normally and read A.
2. Rebuild unchanged normally and require B equals A.
3. Rebuild with --no-cache and require C differs from B.

The marker comes from the real fixture/runtime, and the expected comparisons
are between external observations. Forcing cache=false by default defeats the
equality control; ignoring --no-cache defeats the refresh control. Do not use
timings, image IDs or cache-log wording as evidence that RUN was reused.
This observation proves the profile path. The project path shares the source
boundary but remains without a direct repeat/flag runtime observation unless
the operator separately includes it. The suite's five-case count is unchanged.

The worker runs only formatting, whitespace and source checks. No build, test,
VM, publication or push runs for this cut. Prior a42 results remain evidence
for that head. New-head native compilation and normal-CLI behavior and latency
observations belong to the operator; no speed claim is made here.
