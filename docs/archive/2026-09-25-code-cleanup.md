# 2026-09-25: code-cleanup pass

The whole-tree read the `code-cleanup` skill asks for, run on main at
068dcbc with no open lane. Nine read-only agents, one per module, hunted
yagni, duplication, platform-provided code and restatement; the findings
were judged into three piles and the "land it" pile was landed directly,
one commit per module. Three reworks went to parked heavy tickets.

## Numbers

| Measure | Before (068dcbc) | After (this pass) |
|---|---|---|
| Commits since v0.0.2 | 49 | 69 (16 from this pass; 4 by Adam meanwhile) |
| Lines added / deleted since v0.0.2, whole tree | 4462 / 2481 (ratio 0.56) | 5385 / 4136 (0.77) |
| Lines added / deleted since v0.0.2, `crates/` | 4000 / 2368 (0.59) | 4710 / 3942 (0.84) |
| This pass, whole tree | | 1409 / 2141 (delete/add 1.52) |
| This pass, `crates/pinfold` | | 786 / 1585 |
| This pass, `crates/e2e` | | 382 / 447 |
| This pass, README, ARCHITECTURE.md, `profile/` | | 41 / 92 |
| Largest source file | `crates/e2e/tests/box.rs`, 2982 | 2797 |
| Largest file in the binary crate | `cli.rs`, 1461 | 1208 |
| `docs/ARCHITECTURE.md` | 698 lines | 696 |
| Tickets landed since the last report (Y-70) | 0 | 0 (Y-71 to Y-73 filed, parked) |
| Tickets born from a lane proposal | 0 | 0 |
| Suite wall time, last macOS gate (Y-60) | box 90 s, pi 28 s, cli 0.3 s | box 67 s, pi 25 s, cli 0.3 s (this pass, host run) |

Step 1 found nothing mechanical: the nine dependencies match the spec's
list, clippy with `dead_code` and `unreachable_pub` is clean, there is no
TODO or revisit marker in `crates/` or `profile/`, and all 25 guarantee
rows have a test of the same name. The trace tables found the gaps listed
under "Adam's": three config keys and eight CLI verbs or flags have a spec
line and no test.

Step 2 claimed about 1,650 removable lines across the nine lists before
dedup (three findings appeared in two lists), plus about 550 in `spec:`
items. Landed: 684 net lines in nine commits. Filed: about 430 more in
three tickets. The rest is in "Rejected" with its re-admission condition.

## Landed, one commit per module

| Commit | Module | Removed |
|---|---|---|
| 02a32f7 | config, trust, dirs | The `Containerfile` enum (its two variants were `containerfile_path.is_some()` plus a clone of the profile's bytes); three of four sha256-to-hex loops (`core::hex` survives); trust's four-way "nothing recorded" match (no record is a record of two absences, `Default`); the hand-rolled passwd fallback in `dirs::home_dir` (`std::env::home_dir` is un-deprecated on 1.98); two duplicated error arms in the environment parser. |
| 214cb22 | clean, image, artifacts, profile | Three copies of stage-fill-rename with the concurrent-winner fallback (`dirs::install_dir`; the embedded init moves from `cli.rs` to `artifacts.rs`, where the spec puts it); four "read_dir, NotFound means empty" preludes (`dirs::entries`); the `PI_PINS` table, its lookup and `box_os_arch` (one `pin()` match); the streaming sha256 loop; the epoch text in the maintenance stamp (its mtime is the fact); the clock-near-epoch guard; `load_dir`'s two read matches; `read_seeds`; the walk back from the pi binary path to its directory. |
| 81992d2 | box, plan, network | `open_seed_path` (`seed_file` walks the same descent); the duplicate-guest check outside `Plan::validate`; `PlanError::Json`; the init-is-absolute refusal; `hold`'s second handler install; the owner label inserted by both `Box::up` callers (`Box::up` sets it); in `network.rs`, the hand-rolled IPv4-mapped, IPv4-compatible, fc00::/7, fe80::/10 and ff00::/8 checks (std's `to_ipv4`, `is_unique_local`, `is_unicast_link_local`, `is_multicast`: the same predicates, so the refusal set is unchanged). |
| aa14293 | proxy, tls | The `Allow` struct with its exact/suffix split (one lowercased list, one `allows()`, same matches); the `HeadError` enum (`read_head` returns `io::Result`, a timeout keeps its kind); the version match with an unreachable arm; the content-length arm's own decode-and-push; `tls.rs`'s length helpers whose every caller took the slice they measured (`vec_u8`, `vec_u16`); an unused `Debug`; the stale "no TLS dependency" claim. |
| b36155d | pi | The missing-image refusal built twice (one `ensure_image` names either build command before the run; it costs one extra `image inspect` per profile-image start, unmeasured, in exchange for one message path); the `Stop` enum; `Git`'s root field and accessor; two path-to-str refusals (`utf8`); `project_id`'s `io::Result` over an infallible id; the unreachable `Exact` arm for `GIT_CONFIG_COUNT`; the `any()` dedupe of protected paths; a history comment. |
| 454bd2f | cli, main, build.rs | The per-parser `OsString` conversions and their six "must be valid UTF-8" arms (main converts argv once; `init` keeps its exec path byte-exact); ten usage constants that restated main's `USAGE` table (`usage()` reads the verb's line; `BOX_USAGE` stays); main's working-verb list that restated the dispatch (the verb resolves to a function, then the daily pass runs); `merge_agent_tree` (`copy_tree` takes a skip predicate); `build.rs`'s tool precheck that only rephrased a panic; the `uname` subprocess (nix's `uname`, one feature flag on a listed dependency); `down_line`'s two arms; the image name parsed twice; the profile build's base digest (`image::base_digest`, which image build shares); the owner pid parse and liveness check in both `list` and `clean` (`clean::owner`); `report_image`'s second `resolve_image`; the absent-box exit-3 check in both `exec` and `stat`. |
| 2075993 | runtime | The `io::Result` on `runtime()` for an OS the target table rules out (every caller's `map_err`, doctor's two "no runtime" arms and the daily pass's match with it); four one-caller functions returning an argv literal; `user()`'s duplicated format; the podman module doc's copy of the spec's control list. |
| 5b9fbb6 | e2e | Six copies of run-and-assert `build --profile`; two image-cleanup Drops; three profile Containerfile writes; two host git inits and two status runs; three "image named by reference minus `localhost/`" scans; four line-by-line JSON parses; `labeled_images`'s `(id, name)` pairs that both callers reduced to ids; `box_up_refused`'s copy of `box_up_start`; `Up`'s own child, stdin and stdout beside `Starting`'s; `TestDir`'s Drop (`TestEnv` removes the root); the fixture's second request counter; two copies of "refused run, then plant a cache marker"; three one-caller helpers. Sabotage comments now name `Box::up`'s validate, podman's `rm` argv and `containerfile: None` where the old names are gone. |
| 906fdae | docs, profile | From README: the not-protected list, the Shared files paragraph and Apple table, the credentials paragraph, the `--from-project` sentence (each ARCHITECTURE.md's text again) and the Roadmap (history). From ARCHITECTURE.md: the pi layer's environment bullet restating the harness bullet, and five sentences repeating Terms, Images, Pinned artifacts or AGENTS.md; the default image's list now names `unzip`, which the Containerfile installs. The profile's Containerfile and `pinfold.toml` keep one comment line each; the extension drops its restated rule; `bump-pins.sh` hashes from stdin. |
| 8699e63 | proxy | The hand parse of the request line before httparse's (one parse, both methods; a CONNECT with headers httparse rejects is now a 400); `head_well_formed`'s folded-header pass (httparse refuses obs-fold). A9, A11. |
| 6d85229 | pi | `Shutdown`'s `Option<interrupt>` and second `select!` (SIGINT always ends a run); `hooks_path`'s `git config --get` (`rev-parse --git-path hooks` resolves it as git does). A7, A8. |
| 1b62abf | config, cli, runtime | `Origin`, `Origins`, `scalar_origin`, the provenance block and `config`'s `origins` object; doctor's podman diagnostics (`Detected`, `detect`, `tun_present`, `subordinate_ids_cover`, `IdMap`, `InfoIdMappings`, `docker_present`, `report_podman`, `print_cgroup_manager`) and its prose config and artifacts sections (it prints the verbs' JSON). A1, A2, A3. |
| 480202e | docs | Nothing; the spec names the profile's `share/pi` package and its `skills/` directory. A12. |
| 5a397dd | e2e | Nothing; the four untested keys and verbs Adam kept get assertions, `box_stat` moves to the library, and guarantee 21 asserts GitHub's header. A4, A5, A6, the flaky test. |

Three unrelated commits (f963c8b, 574d189, 6febb04) edited the
`code-cleanup` skill itself while this pass ran.

### Sabotage reruns

AGENTS.md: a commit that touches a test body reruns that test's named
sabotage first. Tests whose body 5b9fbb6 touched, the sabotage rerun (one
per test, two for the cleanup test, chosen nearest the edited lines), and
whether it still bit:

| Test | Sabotage rerun | Bit |
|---|---|---|
| `cleanup_removes_only_pinfolds_garbage` | `keep_two_images` returns before removing | yes |
| `cleanup_removes_only_pinfolds_garbage` | the build sets no source label | yes |
| `every_build_reruns_its_steps` | `--no-cache` dropped from Apple's build argv | yes |
| `a_caller_builds_an_image_from_its_own_tree` | the `--context` argument skipped | yes |
| `a_caller_owned_box_cannot_write_git` | `readonly` dropped from the bind mounts | yes |
| `a_caller_owned_box_launches_the_pinned_harness` | pi mounted at the wrong directory | yes |
| `the_environment_is_exactly_the_spec` | `validate` skipped in `Box::up` | yes |
| `the_box_cannot_write_git_or_protected_config` | the `.git` mount omitted | yes |
| `a_changed_project_file_stops_the_run` | `trust::check` dropped | yes |
| `both_pi_config_levels_load_behind_a_route` | the profile `share/` mount omitted | yes |

One thing the reruns showed: the `.git`-mount sabotage of the pi-git test
panicked while a `pinfold pi` box was up, and the orphaned `container
run` process kept the box alive (and the runner's pipe open) until the
box was removed by hand. That is the spec's fail-closed leftover, which
the next working command's daily pass prunes; under a test harness that
deletes the state dir first it needs `container rm`.

The lifecycle and owner-gone test bodies were left alone (their helpers
changed, their bodies did not) so their nine and four listed sabotages did
not need rerunning; the two findings that would have touched them are in
"Rejected".

## Tickets, parked, `heavy`

| Ticket | Removes | Possible |
|---|---|---|
| Y-71 | runtime: the second `run` argv builder and `HOME` filter, the second digest parser per adapter, the two inspect blocks, ten JSON error closures, one `podman image list` parse | about -150 |
| Y-72 | e2e: 20 "refused with reason" pairs and 18 five-line zero-exit asserts (two helpers), and `FakeModel`, a second HTTP server beside `HttpFixture` | about -220 |
| Y-73 | init: the handler-and-atomic termination loop and the SIGCHLD reaper (`sigwait` and `SIG_IGN`, which the kernel provides) | about -60 |

Each is a rework, not a deletion, and Y-72 touches every test body, so
the lane's review seat and both gates are what the ticket buys.

## Adam's list

Every `spec:` finding and every cut that touches a control, removes a
feature or changes visible output. Nothing here has moved; a yes lands as
an ARCHITECTURE.md edit in the same commit as the deletion.

| # | Finding | Spec line | Lines | Question |
|---|---|---|---|---|
| A1 | `Config::origins` (`Origins`, `Origin`, `scalar_origin`, the provenance block) is read only by `doctor`'s config section and `pinfold config`'s `origins` object. No spec line names provenance; guarantee 18 asserts only `egress.allow`, `trust.ok` and `project.home`. | none | -128 in config.rs, -36 in cli.rs | Is per-key provenance a feature? No: delete it. Yes: a spec line under Configuration, and the `pick<T>` merge (-33) lands instead. |
| A2 | `doctor`'s podman diagnostics (`Detected`, `detect`, `linger`, `tun_present`, `subordinate_ids_cover`, `IdMap`, `docker_present`, `report_podman`, `print_cgroup_manager`) restate what `preflight` already refuses with a named reason; the tun, subordinate-id and docker lines have no spec line. | L84-89, L603 | -132 (keeping linger) | May `doctor` print `preflight()`'s result and the linger line only? |
| A3 | `doctor`'s config and artifacts sections restate `pinfold config` and `pinfold artifacts` in prose. | L603 | -70 | May those two sections print the same JSON those verbs print? |
| A4 | `cpus` and `memory` as config keys: no test sets them through config; the box-spec keys stay. | L572-573 | -48 | Keep (untested), add a guarantee row and test, or cut the config keys and let a pi box take the runtime's defaults? |
| A5 | The configured `protect` list: no test sets it; guarantee 11 covers only the always-protected three. A control. | L60, L535-538, L570 | -34 | Keep (untested), add to guarantee 11's test, or cut? |
| A6 | Spec'd, untested verbs and flags: `attach` and `--box` (97 lines), `profile new --from-project` (about 55), `clean --unused` (about 35), `box exec --tty` and `--workdir`, `image build --no-cache`, `box list --label KEY`. | L541-544, L597, L438-440, L512-513, L120, L125 | up to -190 | Keep as untested features, give each a guarantee row and a test, or cut some? |
| A7 | `pi/launch.rs`'s `Shutdown` carries `Option<interrupt>` and two `select!` blocks so a TTY run does not handle SIGINT. Always handling it would remove the box on a SIGINT during startup instead of killing pinfold. | L300-301 | -18 | Acceptable behaviour change? |
| A8 | `pi/git.rs::hooks_path` runs `git config --file .git/config --type=path --get core.hooksPath`; `git -C ROOT rev-parse --git-path hooks` resolves the value as host git does, including a global `core.hooksPath` that points inside the project, which is then protected too. | L549 | -10 | Yes recommended: strictly more protection, no assertion changes. |
| A9 | The proxy parses the request line by hand, then again with httparse in `parse_plain`. One pass changes the refused set: a CONNECT whose headers httparse rejects (over 64, non-token name) gets 400 instead of a tunnel. | L325, L334-336 | -10 | Yes recommended: stricter. |
| A10 | `tls.rs`'s multi-record ClientHello loop is the parser's only branch no fixture exercises. A single-record read would refuse a fragmented ClientHello. | L325-327 | -8 | No recommended: it is correctness, not bloat. |
| A11 | `head_well_formed`'s folded-header pass: httparse 1.10 already rejects obs-fold in requests, so a folded header still gets 400, logged "malformed request" instead of "ambiguous framing". | L328-329 | -3 | Yes if the log reason may change. |
| A12 | The profile's `share/pi/skills/.gitkeep`, its `"skills": ["skills"]` entry in `package.json` and their embed in `profile.rs` give the profile a user-level skills path the spec does not name; guarantee 14's fixture writes its profile skill there and would move to `home/.pi/agent/skills`. | L456 | -5, one file | Cut the path, or add it to L456? |

### Adam's answers, later the same day

| # | Answer | Landed as |
|---|---|---|
| A1 | Delete. | 1b62abf: `Config::origins`, `Origin`, `Origins`, `scalar_origin` and the two printers gone; `pinfold config` has no `origins` object. |
| A2 | Cut to what the spec names. | 1b62abf: doctor prints preflight's verdict and the linger line; `Detected`, `detect`, `tun_present`, `subordinate_ids_cover`, `IdMap`, `InfoIdMappings`, `docker_present`, `report_podman` and `print_cgroup_manager` gone. |
| A3 | Yes. | 1b62abf: doctor's config and artifacts sections are the JSON `pinfold config` and `pinfold artifacts` print (`config_report`, `pins_json`). |
| A4 | Keep; test it. | 5a397dd: `the_highest_layer_sets_the_allowlist` sets `cpus = 2` and `memory = "1G"` and reads them back through `box stat` and the box's `cpu.max`. |
| A5 | Keep; test it. | 5a397dd: `the_box_cannot_write_git_or_protected_config` names `tooling` in `protect` and shows the write refused and the file unchanged. |
| A6 | Keep untested, except `--unused` and `--from-project`. | 5a397dd: the cleanup test runs `clean --unused 0s` (the live project survives, the other goes); the state test runs `profile new --from-project` (the edited settings arrive, `auth.json` stays behind). |
| A7 | Yes. | 6d85229: SIGINT always removes the box; spec, process supervision. |
| A8 | Yes. | 6d85229: `rev-parse --git-path hooks`; spec, Git. |
| A9 | Yes. | 8699e63: one httparse pass; spec, plain HTTP bullet. |
| A10 | Keep (legal input, cheap). | Nothing. |
| A11 | Yes. | 8699e63: folded headers are httparse's refusal. |
| A12 | Keep; say it in the spec, on the condition that pi loads the package's skills as skills (guarantee 14's test shows that: the profile skill's description reaches the model). | 480202e. |
| Flaky test | Fix. | 5a397dd: guarantee 21's https check asserts GitHub's own response header instead of a 200, since GitHub rate-limits its runners to a 403 and a failed TLS dial would be the proxy's 502 with no such header. |

Sabotage reruns for the tests those commits touched, one per test:

| Test | Sabotage rerun | Bit |
|---|---|---|
| `the_highest_layer_sets_the_allowlist` | `cpus` and `memory` dropped from the plan | yes |
| `the_box_cannot_write_git_or_protected_config` | the configured `protect` list ignored | yes |
| `project_state_persists_and_stays_separate` | every agent entry copied | yes |
| `cleanup_removes_only_pinfolds_garbage` | `--unused` skips the live-box check | yes |
| `an_injecting_route_keeps_the_credential_on_the_host` | the https route never dialed | yes |

## Rejected, with the condition that re-admits it

| Finding | Lines | Why not | Re-admit when |
|---|---|---|---|
| `proxy.rs`: replace the `TunnelSocket` trait by `TcpStream::from(OwnedFd)` over the unix clones | -30 | Types a unix socket as TCP; a footgun for a reader | std gains a socket trait, or a nix `setsockopt`/`shutdown` form is shorter than the trait |
| `proxy.rs`: build the TLS config always instead of only with an https route | -6 | Adds `load_native_certs` to every `up`, unmeasured | A measurement shows it under 5 ms |
| `profile.rs`: extract the whole embedded profile to the cache and load it through `load_dir` | -34 | Moves the default profile into a cache dir `clean` must then know; six file reads per command, unmeasured | A second embedded-profile bug appears, or `clean` gains a cache category anyway |
| `box.rs`: one teardown (`abort`) for `down` and `hold`, dropping `Stop::NotReady` | -18 | Changes teardown order after ready (kill the client before `runtime.down`); Apple's tolerance unverified | A box.rs rework ticket with both gates as proof |
| `box.rs::down`: a blocking flock instead of the 1000 by 10 ms poll | -8 | Drops the unspecified 10 s cap on waiting for the owner | A spec line says `down` waits unbounded |
| `plan.rs`: drop the `image == ""` refusal | -3 | Apple `image inspect ""` behaviour unverified | A check on Apple shows `resolve_image("")` refuses |
| `podman.rs`: drop `rfc3339` and `civil_from_days` for `ps`'s own `CreatedAt` | -23 | `CreatedAt`'s shape on the pinned podman unverified | A labvm check shows an RFC 3339 string |
| `launch.rs`: drop the `Spec` arm that strips core's "spec: " prefix | -3 | pi.rs asserts the prefix: a test assertion change | That assertion is rewritten for another reason |
| e2e: `TestDir` as a bare `PathBuf` (its Drop is gone) | -10 | 60-site churn for 10 lines | Those sites change anyway |
| e2e: `runtime_image` in the lifecycle body, `json_lines` and `box_prune` inline in the owner-gone body | -12 | Those bodies list nine and four sabotages to rerun | Those bodies change anyway |
| `.dockerignore`: Yard's suggestion boilerplate and commented entries | -9 | Whether Yard re-appends its block unverified | A check shows Yard leaves the markers' content alone |
| `operating-context.ts`: `.split(",")` for `.split(/[\s,]+/)` | 0 | No line saved | Never, on its own |
| `launch.rs`: drop `canonical()` over `git --show-toplevel` | 0 | No line saved | Never, on its own |

Speedups: none proposed with a before-number; none landed.

## Spec pass

Guarantees table against the tests: 25 rows, 25 tests, one each; no row
changed. Open questions: none of the four is answered by the suite, so
all four stay. Restatements cut are in 906fdae. Adam's answers landed
the same day (the table above); the guarantees table still has one test
per row, and no row changed. No release tag was requested with this
pass.
