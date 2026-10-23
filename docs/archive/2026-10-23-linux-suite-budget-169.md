# Linux suite budget: fewer guarantees (#169), 2026-10-23

vm-boxes CI run 40002315497 passed every test but measured 301.5 s
against the 300 s budget. Same-code runs on vm-boxes took 214.8, 245,
289.7, 295 and 297.1 s, so runner noise alone crosses the line.

## Where the time went

The Rust suite runs four tests at a time on four vCPUs. Their summed
durations divided by four match the end of the shared phase (170 s fast,
238 s slow), so the threads are full and only removing work helps. A
serial tail of four `runtime::` tests adds 44–59 s. From a timing run
(40004276520, fast runner): 67 box starts took 169 s, 16 `pi --version`
runs 100 s, 12 pi RPC starts 65 s, profile builds 58 s, and one second
`cargo build` of pinfold 19 s.

Most expensive tests, slow / fast runner, seconds: row 14 103/72, row 30
85/54, row 12 74/47, row 9 62/54, row 26 60/56, row 6 53/34, row 32 50/38,
row 19 48/41, row 13 45/29, row 28 41/18, row 31 30/19.

## What went

Adam approved cutting low-value guarantees in chat; the exact controls
keep their tests.

- Row 30 (changed image inputs prompt a rebuild): a convenience hint.
- Row 31 (changed bundled profiles are reported once): a convenience
  notice. Its second `cargo build` also served row 28, which now gets its
  different valid executable by appending bytes to the suite's binary.
- Row 32 (the documents profile reads documents locally): its image build
  alone took 34 s and it ended the shared phase.
- Row 14 keeps one routed `pi -p`: profile and project skills and the
  host's allowlist reach the model. The default/full/default switch, the
  background `full` build and the saved-settings check went.

The features stay in the spec; only their tests went. CI now builds
neither the `documents` nor the `full` bundled image, so a nightly pin
bump to either Containerfile reaches no e2e build.

The estimate was 170 or more thread-seconds, about 45 s fast and 55–60 s
slow. The PR's CI runs give the measured result.
