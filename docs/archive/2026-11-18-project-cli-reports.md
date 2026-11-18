# Project CLI reports on macOS

Base 865debb. The operator requested readable reports by default, explicit
JSON for scripts, and a shorter first screen.

The native debug build and the previous installation ran in a disposable Git
checkout with private XDG directories and the existing default image. All
stdout checks used pipes. The CLI produced the following results.

- Bare `pinfold` showed Pi and shell examples. `--help` listed the full
  command groups. The removed `artifacts` verb exited 1.
- `doctor --artifacts` printed names, versions, cache status and paths.
  Parsed `doctor --artifacts --json` equalled the old `artifacts` array.
- `config` in an unconfigured project exited 1 with no-config. After
  `config edit`, it displayed the folder, share, profile, local image status
  and settings path. Parsed `config --json` equalled the old config object.
- `doctor` led with findings status. `doctor --json` emitted one JSON object.
- Share fetch refused as share-no-crossing. With an explicit disk project
  and a kept owner, push and fetch printed receiving refs, commit ids and
  transfer direction. Each parsed --json result equalled the previous
  binary's record from the same owner.
- `-- sh -c` emitted the supplied multi-line text on stdout and a separate
  marker on stderr. Both matched the expected bytes.
- Caller image build from a local base printed its tag. Image rm printed
  its removal result. With --json both returned their built/removed events.

The fixture stopped its owner and removed its disks, image and temporary
directories. Cargo fmt --check, cargo build --locked, cargo clippy
--all-targets --locked -- -D warnings, and bash -n scripts/slow-proxy.sh
passed. Linux runtime behavior was not exercised in this check.
