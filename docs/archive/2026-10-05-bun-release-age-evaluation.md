# Bun release-age evaluation

Evaluated Bun 1.4.2's `minimumReleaseAge` as a replacement for the npm
selection in the nightly updater (now `scripts/bump-pins.py`). Kept the
current implementation. The built-in feature does not satisfy the
bundled-tool policy on its own, and the additional validation would leave
two overlapping resolvers.

## Evidence

The [official installation documentation](https://bun.sh/docs/pm/cli/install#minimum-release-age)
states that missing publication timestamps pass the gate, existing lock
entries stay unchanged, and range or dist-tag resolution can apply an
additional rapid-release stability filter. Exact version requests enforce
age but bypass that stability filter.

Eight disposable registry scenarios exercised the real host Bun executable,
with isolated configuration, fresh caches, ignored scripts and a seven-day
threshold. These are evaluation fixtures, not pinfold binary tests.

| Scenario | Observed behavior |
| --- | --- |
| New version without a publication timestamp | Resolution succeeded and locked the version. |
| Exact version published two days ago | Resolution failed. |
| The same young version already locked | Install succeeded without changing the lock. |
| Young tagged latest and older eligible version, with a higher mature untagged version | Selected the older version below tagged latest. |
| Mature parent, mature ARM native dependency and young x64 optional dependency; ARM target | Succeeded and omitted the x64 package from the lock. |
| The same packages, both Linux architectures requested | Also succeeded and omitted the young x64 package. |
| Both native packages declared as required direct dependencies | Failed on the young x64 package. |
| A newer mature parent with a young optional native package, and an older complete mature parent available | Succeeded with the newer parent and omitted the young native package; it did not fall back to the older complete parent. |

The last scenario is decisive for AnyDoc. Its native binaries are optional
in npm metadata, but pinfold requires matching GNU packages for both Linux
architectures. The updater must select a complete eligible parent release,
not treat an omitted platform binary as success. Bun's
[optional dependency failure handling](https://github.com/oven-sh/bun/blob/bun-v1.4.2/src/install/PackageManager/PackageManagerResolution.rs#L329-L337)
permits that omission.

Also resolved the real packages from npm using:

```sh
bun install --lockfile-only --ignore-scripts --no-cache \
  --minimum-release-age 604800 --os linux --cpu '*'
```

The temporary manifest requested `@dietrichgebert/ponytail@latest` and
`@firecrawl/anydoc@latest`. Bun selected ponytail 4.10.0 and AnyDoc 0.2.4.
Frozen installs targeting Linux ARM64 and x64 both succeeded and kept the
lockfile byte-identical. Each installed its GNU and musl native packages.
This verifies cross-platform package selection and installation on the
host, not execution of the Linux binaries on macOS.

Live metadata showed ponytail had no runtime dependencies and AnyDoc had
only its exact-version optional native packages. There is no general
runtime dependency tree for Bun to simplify in the current bundle.

## Decision

Retain direct verified archive installation and the existing npm resolver.
Replacing it would still require strict publication-time validation,
checking both native packages, selecting an older complete parent when
necessary, and preserving existing versions and checksums. It would also
add updater Bun provisioning, temporary lockfile handling and extraction
of resolved metadata. GitHub release and asset age checks remain separate.

Bun's feature is appropriate for users who choose Bun-managed profile
packages and accept its documented semantics. User profiles remain
user-managed; no global Bun configuration was changed.

No implementation or spec changed during this evaluation. Earlier
document conversion and skill-loading E2E results still apply. Rust and
runtime tests were not repeated for this documentation-only result.
