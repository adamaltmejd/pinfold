# Pinfold diagnostics and integration architecture

Research date: 2026-10-15. Decision report; no production implementation.

**Recommendation: retain the process boundary and make it a deliberate public contract. Repair consumers now. Add binary-owned, generation-specific diagnostic facts and bounded read operations next. Do not introduce a public SDK, embeddable core library, multiplexed RPC server, or persistent service at this stage.**

A diagnostics command alone cannot answer the questions: configuration provenance and several upstream failure causes are discarded before any command could retrieve them. Conversely, a daemon or SDK would not recover those facts. The useful redesign is the representation and lifetime of evidence, together with reliable subprocess supervision. It leaves security enforcement and box ownership in the binary.

Approve or reject the contract and staged scope below. The disposable code is evidence, not an implementation candidate.

## Baseline and method

Requested checkout: `<checkout>`, branch `codex/issue-89-bounded-listing`, HEAD `198600b1751d889d354ed3657d8f839e8436e469` (0.2.3 workspace). Read its AGENTS.md and binding ARCHITECTURE.md. Existing dirty work affects README, CLI, box lifecycle, clean, runtime implementations, E2E box tests, architecture, and Linux runner setup: 484 insertions and 108 deletions across ten tracked files. Numerous existing investigations/archive files are untracked. None was changed by this research.

The baseline includes those uncommitted changes, notably 20-second cancelable box inventory, runtime version admission, and per-start Linux seccomp selection. It is not pristine HEAD. The baseline patch and reconstructed source manifest (not retained) identified the source. The initial manifest filename was accidentally reused by the baseline runtime receipt. To recover tracked-source provenance, I reversed the complete prototype patch against the isolated source copy and compared every tracked file with the primary checkout: zero differences. This is a reconstructed manifest, not an untouched initial receipt or proof about historical untracked-file contents.

Probe fixtures set a private maintenance timestamp to prevent Pinfold’s automatic host-wide maintenance from touching unrelated resources. This is an experimental safety accommodation using a known internal file, not a proposed consumer dependency; a production integration must not require it.

All implementation experiments were in an ordinary temporary source copy, not the linked working tree and not the primary checkout. The existing restriction on guest access to linked Git worktrees remains. A second agent traced consumers; another traced core facts; a separate researcher reviewed current official comparator material and independently challenged the recommendation. Builds and runtime experiments were coordinated in the main session. No production code, tests, spec, dependencies, commits, releases or infrastructure services were changed.

Runtime evidence:

- macOS 27.0.1 arm64, Apple Container 1.5.0. Baseline rebuilt from the copied dirty source; prototype rebuilt from it. Real Pi 1.2.0 RPC sessions answered `get_state`; no model account or generated model response was needed.
- Yard Linux x86_64, rootless Podman 6.1.4 with systemd cgroups. Baseline used installed Pinfold 0.2.2, SHA-256 `7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`. Prototype was cross-built from this research's dirty 0.2.3 source. Linux baseline results are not a same-source comparison.
- Existing images were reused. Each readiness receipt records its actual image ID. The shared macOS default tag changed between experiments, so these are functional observations, not controlled timing or image-performance comparisons. This research built no images and did not alter the shared builder.
- Linux isolation initially hid the user's containers/storage configuration, producing a real preflight refusal. Explicitly referencing the existing storage.conf restored the same rootless store without changing it. The failed setup receipt is retained separately.

The prototypes use real binaries, real runtimes, actual CLI interfaces and outside observations. No mock runtime, binary-only test switch, internal unit test or speculative production test was added. Formatting and Clippy passed for the scratch source. Both host and Linux builds succeeded; the cross-build emitted a linker warning, retained in its log. The full guarantee suites, Linux arm64, crash-durable storage and prolonged load were not run.

## Workflows and where information is lost

| Practical question | Current evidence and problem class | Required owner of the fix |
|---|---|---|
| Why did startup fail? | `box up` already emits refused/reason/detail. Pi Durable drops detail; its helper discards stderr. Other late failures are detail-only. Consumer loss plus incomplete interface. | Consumer repair; typed binary failure stage/code. |
| Why was this request refused? | Existing JSON egress log distinguishes local denial from routed upstream status. Humans must locate/read it. Some DNS/dial/TLS/response failures lose their causes. Presentation plus binary fact loss. | Binary records causes; host/Pi presentation renders them. |
| Which config source supplied a setting? | `Layer::over` resolves values but drops the winning source. Values re-read later cannot reconstruct provenance. Fact loss. | Config merge and launch snapshot in binary. |
| Which policy applied to this box? | `config` describes current files/environment. It is not the policy of an existing run. Runtime labels already include generation, but terminal/stat/log associations are incomplete. | Generation-bound immutable launch facts. |
| What happened before termination? | Owner stdout has normal terminal event; successful teardown deletes transient state. SIGKILL emits no terminal event. Egress survives, but name reuse mixes lifetimes. | Retained observations, explicit incompleteness; never invented death cause. |
| Is a null/empty result meaningful? | Apple unsupported measurements and Linux missing/failed/unlimited measurements collapse into null. Absent historical evidence does not prove no event occurred. | Per-section/field availability and coverage. |

Source locations in the inspected baseline:

- [CLI process output](../../crates/pinfold/src/cli.rs): `up`/`hold_up`/`down` around 205–318; raw `exec` and absent exit 3 at 321–356; `doctor` 622–712; `config` 728–775.
- [Configuration](../../crates/pinfold/src/config.rs): load 54–115, replacement merge 182–191. [Trust](../../crates/pinfold/src/trust.rs): checks already-loaded bytes, 45–66 and 135–143.
- [Box lifecycle](../../crates/pinfold/src/core/box.rs): effective preparation 172–239, generation label 264–267, owner hold 341–354, name-based egress path 547–552. [Ownership](../../crates/pinfold/src/core/ownership.rs): generation 211–229, teardown cleanup 280–301, generation-addressed stop 321–329.
- [Proxy](../../crates/pinfold/src/core/proxy.rs): append 65–70, CONNECT order 242–280, HTTP/route handling 286–354, erased TLS error 381–388, response handling 472–535, dial errors 600–605, audit fields 737–742. [Network](../../crates/pinfold/src/core/network.rs): all-address DNS rejection 22–35.
- [Runtime common](../../crates/pinfold/src/core/runtime/mod.rs): unbounded generic capture 93–105; non-TTY exec process group 388–428. [Apple](../../crates/pinfold/src/core/runtime/apple.rs): unsupported stats 87–105; [Podman](../../crates/pinfold/src/core/runtime/podman.rs): conflated stat failures and `max`, 268–280.

`config` and `doctor` also synchronously contact runtime subsystems before finishing host facts. The bounded box-list change does not bound image inventory/preflight. A diagnostic path must not wait for that work to return local retained facts. This is source evidence; no stalled daemon was induced.

### Real consumers

Pi Durable's [`prototype.ts`](../../examples/pi-durable/prototype.ts) helper counts both output streams but retains only stdout (72–89). Startup owner stderr is inherited, **not universally discarded**. Startup errors omit `detail` (143); returning at `ready` and closing readline (142,150) discards later lifecycle observations. Its no-egress configuration supplies no evidence for network-diagnostic demand. Repairing it would improve actual failures without changing the binary.

Interactive Pi uses core functions directly, through [`pi/launch.rs`](../../crates/pinfold/src/pi/launch.rs#L226). It should render the same typed core observations rather than spawn its own CLI or acquire a second policy implementation. Its guest operating-context extension knows an allowlist and that a 403 is final; that is not a complete explanation of routes, source provenance or upstream errors.

A current real external controller was available on GitHub: [Switchyard box.rs at 81da8c6](https://github.com/adamaltmejd/switchyard/blob/81da8c677a0ab43966588429c24a8affedb9e8ec/crates/yard/src/box.rs). Unlike Durable, it drains lifecycle continuously and keeps startup detail. It nevertheless parses stderr prose to distinguish absent exec (303–315), collapses unsupported/absent/failed OOM reads (319–324), and retains terminal reason while dropping detail (451–466). [Supervision](https://github.com/adamaltmejd/switchyard/blob/81da8c677a0ab43966588429c24a8affedb9e8ec/crates/yard/src/jobs/supervise.rs) samples OOM before teardown and refuses to trust a candidate when cleanup fails. Those are demonstrated source workflows, not invented scheduler requirements. The 600-line box module contains controller policy as well as transport; it is not 600 lines an SDK could remove.

## Experiments and discriminating results

Raw receipts and small drivers are in [the evidence directory](2026-10-15-diagnostics-evidence/README.md). Results below distinguish the unchanged baseline, repaired consumer, and modified binary prototype.

| Scenario | Result | Architectural consequence |
|---|---|---|
| Real route success versus local denial versus upstream 403 | Both runtimes returned 200 from the host route fixture; denied `example.com` returned local 403 plus `not allowlisted`; the fixture's 403 logged `allowed/route/status:403`. | Explain decision origin. HTTP 403 alone cannot say who refused. |
| Unavailable upstream | Linux returned 502 with `allowed/route/status:null`. macOS client's five-second deadline expired before a route entry appeared; teardown ended the observation. | No claim that a timeout proves a policy denial, or that absent log means no attempt. Existing records cannot establish exact connect/TLS/exchange cause. |
| Exit 3 | Guest printed exact `raw-out` and `raw-err`, exited 3. Absent box also exited 3, with Pinfold prose stderr. | Keep raw streams/exits; separate supervision metadata is needed for unambiguous classification. |
| Same-name reuse | Baseline second generation saw the first generation's log before making any request. Prototype second generation started with an empty separate log. | Promote existing generation to a documented run handle and bind history to it. |
| Startup refusal | Missing image yielded reason `image-missing` and useful detail; repaired consumer retained both. | A consumer repair has immediate value. It cannot create finer binary reasons. |
| Owner SIGKILL | No terminal event; orphan still answered stat until targeted down. Prototype retained ready facts but terminal was `not-observed`, liveness `not-probed`. | Retained ready is historical evidence, not current health. Exact death cause cannot be reconstructed. |
| Closed owner stdout before ready | Baseline and prototype exited 101 and left a running orphan on both runtimes; exact-name down removed it. | The readiness-output cleanup ticket (landed as 7feaf89) is a prerequisite to reliable ownership handoff. Prototype deliberately did not fix it. |
| Adapter owner death with inherited pipe | Initial TypeScript adapter hung waiting for Node `close`: a runtime descendant retained stderr. After revision, owner exit and bounded pipe drain are separate; result says `outputDrain:deadline`. | A thin wrapper still needs careful lifecycle engineering. EOF is not process exit. |
| Bounded command capture | Repaired adapter delivered all 200,000 stdout bytes and three stderr bytes separately, retained a 65,536-byte tail, marked truncation, preserved guest exit 3. | Buffering policy belongs in transport/presentation, with explicit loss. This does not prove arbitrary slow sink behavior. |
| Observer cancellation | Pre-aborted list observer ended; a subsequent guest command returned `still-alive`. | Observer cancellation must not close owner stdin or call down. |
| Real Pi configuration edit | Pi answered RPC readiness. Config changed allowlist from `api.github.com` to `example.com`; current config showed new value and failed trust, but attached request remained denied under old policy. Prototype snapshot/check still identified the old policy. | Current configuration is not applied policy; later trust failure does not retroactively stop an existing run. |
| Bounded retained query | Limit 1/3 returned the requested prefix with truncation; prototype queries also worked with PATH pointing to no runtime executable. | Read-only local evidence can be independent of runtime executable availability. This does not prove behavior under a stalled service or filesystem. |
| Unexpected runtime termination | Targeted runtime SIGKILL produced owner `down/exited`, detail `exit status:137`, exit 1. Snapshot survived. Prototype terminal retained only `exited`. | The proposed implementation must preserve structured exit status; the prototype still loses useful terminal detail. |

The macOS non-reading observer test returned only 13,726 bytes and completed before cancellation; it did **not** establish backpressure isolation. A separate Linux probe constrains the actual output pipe to 4 KiB to test a blocked reader. Its outcome is recorded below in the final verification section.

### What each prototype actually establishes

**A: repaired consumer over the existing binary.** A 168-line TypeScript reference adapter keeps bounded lifecycle/stderr, preserves startup detail, separates raw exec streams, distinguishes owner exit from pipe EOF, and cancels an observer independently. It contains generic JSON records, not a fully validated/versioned typed SDK. Its stop result is process completion, not a verified teardown receipt; the experiments verify removal separately. Raw exec can still wait for descendant-held streams, and synchronous output callbacks can throw or block. It proves transport repairs are useful and affordable. It does not solve absent facts, provenance, same-name history, exec classification, arbitrary slow callbacks or per-command cancellation. Its first failed implementation is preserved as a research result, not concealed.

**B: retained facts plus a coherent query boundary.** The [disposable patch](2026-10-15-diagnostics-evidence/prototype-b.patch) records selected resolved facts at ready, keys egress by existing generation, records coarse terminal information, and adds `box inspect-run` and `box check-run`. Queries read at most 64 KiB per snapshot/terminal/log and at most 100 events; a shared helper performs authority policy checks for enforcement and explanation. It does not record complete configuration provenance, startup stages, all runtime controls, detailed upstream errors, cursors, or retention quotas. Files are bounded per read, not durably immutable or bounded in aggregate. Permission/symlink/durability hardening is not implemented. Best-effort snapshot writes are not an added fail-closed security control.

Representative real controls passed after sharing static checks; source review found the same CONNECT/HTTP ordering. This is not proof of full policy equivalence and is not authorization to ship the extraction. The patch changes a control path in scratch; production adoption needs the relevant existing guarantee scenarios and full required gates.

The strongest alternatives were these two: consumer-only and binary contract plus consumer. A service/library prototype would not discriminate the observed missing-fact problems; neither concrete controller needs multi-host RPC or multi-client lifetime ownership. A separately packaged SDK would wrap the same adapter and add packaging/version support without changing any observed result.

## Alternatives and cost

| Alternative | Correctness and usability | Integration effort and maintenance | Dependencies and operations | Decision |
|---|---|---|---|---|
| Consumer repair and isolated CLI additions | Recovers discarded facts; insufficient for provenance/history/outcome taxonomy unless binary also changes | Smallest immediate repair, but ad hoc schemas accumulate | No new runtime dependencies/service | First stage only |
| Deliberate subprocess contract with retained run facts | Addresses demonstrated identity/evidence gaps; raw exec and stdin ownership remain | Binary schema and compatibility discipline; controllers still supervise processes | Existing Rust dependencies, small host-owned files; no daemon | Recommended target |
| Thin language SDK over that contract | Useful ergonomics; cannot improve missing facts by itself | One TS/Bun consumer and one Rust controller do not share a language; release/type/transport testing added | Optional library distribution and version matrix | Reference adapter now; public package only after repeated demand |
| Public embedded core library | Interactive Pi already calls core internally; external embedding couples controller to binary internals and versions | Public Rust API/ABI expectations, signal/runtime ownership complexity; no help to Bun without FFI | Build/link coupling, possible FFI dependencies | No demonstrated benefit |
| Persistent service or multiplexed RPC | Could support remote clients and resumable observation, but cannot reconstruct missing facts | Authentication, service lifecycle, reconnect/backpressure/upgrade protocols; temptation to transfer ownership | Additional long-lived authority and operations | Reconsider only for demonstrated remote/multi-client workflow |

A service could be the right later architecture if a remote controller must survive client loss while deliberately preserving boxes, or many independent clients need durable subscriptions. That is a different lifetime contract. Do not obtain it accidentally by keeping stdin alive in an SDK background worker.

### Comparator evidence

Reviewed current official documentation and pinned source/published client code, not comparator runtime behavior. Full links, revisions, qualifications and code-level observations are in [comparator evidence](2026-10-15-diagnostics-evidence/comparators.md).

- [Docker monitoring](https://docs.docker.com/ai/sandboxes/governance/monitor-and-enforce/monitoring/) exposes effective rules, origins and decision reasons. Its published cloud SDK 0.53.0 bounds capture, keeps draining, marks incomplete output and preserves a process resource on uncertain waits. Local `sbx` policy and cloud API are separate surfaces; no claim of one unified tested implementation.
- [Matchlock SDK source](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/sdk/client.go) wraps a binary using JSON-RPC, showing that an SDK need not replace the process boundary. Its reader discards subprocess stderr and invokes sinks synchronously: a wrapper can reproduce the very diagnostic loss being fixed. Source-derived risks, not tested failures.
- [E2B command handle](https://github.com/e2b-dev/E2B/blob/62a17dfc3f11439d1d06bfb9bb515eadc5adfcd3/packages/js-sdk/src/sandbox/commands/commandHandle.ts) separates disconnect, kill and command exit, but accumulates full output even with streaming callbacks. Its [lifecycle events](https://docs.e2b.dev/sandbox/lifecycle-events-api) tie history to execution identity.
- [Daytona errors](https://github.com/daytona/clients/blob/82b53c9cbb4abb8a0505411e8b15c01085d3b94a/sdk-typescript/src/errors/DaytonaError.ts) preserve machine code and origin and distinguish client timeout from server completion. Simple exec combines output, while session APIs separate streams. Current core is private; only public clients/docs were inspected.

Borrow explicit identity, error origin, partial results and bounded capture. Do not copy cloud lifecycle, a policy AST, SQLite allocation machinery, or a WebSocket service without its motivating workflow.

## Proposed minimum public contract

These are proposed semantics, not existing commands or a tested finished schema.

**Identity and stages.** `run_id` is the existing generation, not a new ID system. Ready, terminal, stat, diagnostic and network records carry it. Name remains a convenience for selecting the current run. Historical access requires run_id. Early refusal before a name claim has `run_id:null` and creates no state, preserving current admission semantics. After claim, distinguish `preparing`, `ready-observed`, and `terminal-observed`; never label an intended plan as successfully enforced. Generation-conditional mutations must compare the supplied expected generation under the ownership mechanism, not list then mutate by name.

**Launch snapshot.** Capture facts from the already-loaded/checked data used to start the box, not by reading config again. Include resolved image ID, Pinfold/runtime version, selected profile identity, effective nonsecret allow/routes/resource/protection settings, each setting's winning layer, and trust inputs. Lists and routes replace as whole settings today: provenance follows that replacement unit. Built-in defaults, profile, trusted project and environment are distinct source kinds. Environment provenance names the variable only. Core callers' policy source is `box-spec`; core must not invent Pi-style file provenance.

Persist only an explicit allowlist of fields. Never serialize Plan wholesale, env values, injected header values/prefixes, raw TOML, command argv/output, login helper output or arbitrary error chains. Trust digests/source hashes need review for secret-bearing inputs; use the existing trust representation only where safe, not public hashes of individual secrets. Snapshot states which configuration the binary applied/prepared, not independent attestation that every kernel control worked. Credential freshness, DNS and current runtime health are separate observations.

**Reasons and availability.** Structured results have schema major, operation, stage, stable code, relevant run_id, observation time and human detail. Use narrow enumerated facts such as `not-allowlisted`, `runtime-timeout`, `upstream-connect-failed`, `upstream-tls-failed`, `upstream-response-invalid`. Do not stabilize raw OS prose as codes. Preserve observed exit status and signal separately from why supervision ended and whether teardown completed.

For each independent section/measurement use `available`, `unsupported`, `unavailable`, `not-observed`, or `not-applicable`; represent unlimited explicitly. Include source/time and truncation/coverage. A failed fresh read may include an explicitly dated last observation, never silently substitute it. Unknown historical run is unavailable, not proof it never existed. Only call it expired/pruned if retained evidence establishes that. An empty event page proves no matching records within its reported coverage, not no traffic or successful delivery.

**Runtime-independent history and bounded queries.** Start with finite one-shot inspection and paged events, not a live subscription server. Retained metadata and event reads must not call runtime inventory. Optional live stat/doctor probes have their own deadline and return partial results; their failure cannot suppress local config/trust/history. Proposed limits: 64 KiB per metadata document, at most 100 events/64 KiB per page, an opaque cursor bound to run and ordering, explicit next cursor/truncation/coverage. New reason fields must have fixed size limits, with untrusted detail bounded and escaped. Runtime-dependent queries use a stated total deadline, not merely a caller's unbounded wait.

For retention, keep the existing 14-day horizon as the initial policy and add a proposed 128 MiB aggregate quota for **inactive diagnostic history**, evicting oldest inactive history first. Document that quota can shorten retention. Never truncate or drop an active mandatory audit append to meet this quota: the existing append-failure shutdown remains exact. Active logs remain potentially unbounded until a separate disk-containment decision; query bounds do not solve storage exhaustion. After eviction, report unavailable with retention coverage, without manufacturing a specific tombstone for every run. Age/quota cleanup must be independent of querying and must preserve live ownership. These limits are proposed defaults to approve and test, not measured capacity requirements.

Filesystem reads can themselves stall; isolate finite query processes and allow cancellation, and state the limit of uninterruptible OS I/O. The prototype's PATH test does not establish a wall-clock guarantee for all storage/runtime failures.

**Cancellation and output.** `up` continues to own one box; stdin EOF ends it. Drain its stdout/stderr throughout life. Treat process exit, pipe EOF, terminal event and confirmed teardown as distinct facts. Bounded post-exit draining may mark incomplete output, but must not infer cleanup. Explicit owner cancellation follows existing signal/down semantics and reports unconfirmed teardown on deadline; no automatic hard kill plus success. Canceling a query stops only that query. Guest work has whole-box cancellation today; canceling an exec wrapper does not promise to kill the guest command or its background children.

`box exec` keeps raw stdout, raw stderr and exit behavior. For the demonstrated exit-3 ambiguity, add a narrowly scoped opt-in metadata channel, for example `--diagnostic-fd 3`, only after designing inherited-FD handling for both platforms. Emit bounded host supervision records there, never guest output or credentials. Metadata absence/EOF is unknown, not success. A terminal host record distinguishes dispatch refusal from an observed guest status; its absence cannot establish whether work began. No caller should infer absence by parsing command stderr. This is an unprototyped part of the proposal and a separate implementation item, not justification to encode all exec bytes in JSON-RPC.

**Policy explanation.** Reuse the enforcement decisions in the binary. A static check for a named run can establish protocol/authority/port/literal/allowlist/route selection in the same precedence as enforcement. It cannot validate an unprovided HTTP framing or TLS ClientHello. Positive result: `passes-static`, with DNS, SNI, credentials and delivery unevaluated as applicable. CONNECT routes are refused; HTTP routes can pass without a public allowlist match. HTTPS route targets require checked DNS; host-service HTTP routes intentionally use different address rules.

Snapshots must name the policy-evaluator version. If a later binary cannot evaluate that version, static checking returns `unsupported-policy-version`; it must not silently reinterpret old policy with new rules. Keep recorded observed decisions readable without maintaining a compatibility policy engine. The prototype does not implement this version check. A static refusal is counterfactual, not evidence that a request occurred.

Optional `--resolve` would use the existing all-address validation with an isolated deadline, report a timestamp and checked DNS outcome, and make no HTTP request. It cannot bind a later request to the same resolution. Do not promise DNS checking in the first implementation: it was not prototyped. Observed policy admission, connection establishment, TLS handshake and upstream HTTP status are distinct facts. A permitted request may fail later. CONNECT reveals authority and SNI, not encrypted paths/status or originating command; do not add attribution, interception, request capture, method/path authorization, automatic allowlisting or a generic policy language.

## Human and programmatic use

The prototypes actually exercised host inspection while Pi ran, and retained-policy queries after it ended. A human renderer of those real results is included with the evidence. The proposed product interface could be:

```text
pinfold diagnose --project .                 # select this project's current/recent run
pinfold box inspect RUN_ID --json            # local retained facts, no runtime probe
pinfold box events RUN_ID --limit 50 --json   # bounded page + coverage/cursor
pinfold box check RUN_ID --scheme https --host api.github.com --port 443 --json
```

Example human presentation, with provenance shown only once actually captured:

```text
example.com: refused by this run's proxy (not-allowlisted).
Applied allowlist: api.github.com. Source: trusted project .pinfold.toml at launch.
Current file differs and is not trusted. It did not change this running box.
Last observed outcome: HTTP 403 from the proxy. No upstream request was authorized.
```

For an allowed authority: “Static policy permits this authority. DNS, TLS/SNI and delivery are not established.” For owner loss: “No terminal event was recorded. Last ready observation is retained; current liveness has not been checked.” Never say “OOM” from an unsupported/null measurement or “request succeeded” from an allowed log entry.

Pi should print a concise host-side explanation after a launch failure or unexpected end, plus a host command to inspect the recorded run. While Pi runs, a second host terminal can query it. Do not put a host control socket in the guest or teach a guest extension to read host-private files. A future `/diagnose` bridge would require a separate, narrow host/guest information boundary and concrete UX evidence; it is not needed for this scope. The experiment used Pi RPC and host attach, not a finished interactive TUI extension.

A direct external caller remains straightforward in outline (proposed schema, abbreviated error handling):

```ts
const owner = spawn(pinfold, ["box", "up"], { stdio: ["pipe", "pipe", "pipe"] });
owner.stdin.write(JSON.stringify(spec) + "\n"); // keep open until explicit stop
// Continuously drain bounded lifecycle and stderr; retain ready.run_id and terminal.
// Observe owner 'exit' separately from stream EOF; record incomplete drain if needed.
const query = spawn(pinfold, ["box", "inspect", runId, "--json"]);
// Cancel query only. Owner lifetime is unchanged.
const exec = spawn(pinfold, ["box", "exec", name, "--diagnostic-fd", "3", "--", ...argv],
                   { stdio: ["pipe", "pipe", "pipe", "pipe"] });
// Forward stdout/stderr unchanged; read host metadata on fd 3; keep actual exit code.
// Stop explicitly through owner signal/stdin and wait for confirmed teardown.
```

The retained [adapter](2026-10-15-diagnostics-evidence/consumer.ts) demonstrates more of the necessary handling than this outline, but deliberately has no production SDK promise. A TypeScript/Bun reference adapter is warranted for Pi Durable. A public TS package is warranted only if a second maintained TS consumer needs the same transport and its semantics stabilize. Switchyard is Rust and should consume the same public schema directly; do not publish a Rust library merely to remove JSON parsing. If later packaged, an SDK owns spawning, byte framing, schema validation/version checks, bounded buffering, process/stream termination, explicit cancellation and typed results. It must own no policy merging, trust decisions, DNS authorization, secret resolution or independent cleanup authority.

## Migration and smallest useful implementation

1. **Repair existing information loss and readiness cleanup.** Durable retains stderr/detail, drains after readiness and handles process exit separately from pipe EOF. Switchyard retains terminal detail and stops flattening unavailable measurements. Fix the already-written readiness-output cleanup ticket in the binary. Preserve current raw exec semantics. Consumer fixes still cannot solve exit-3 ambiguity; leave that uncertainty explicit until metadata exists.
2. **Add retained run facts and honest read access.** First-class existing generation, prepared/ready/terminal stages, safe selected policy fields and provenance captured at merge time, structured availability, finite query bounds, inactive retention policy and generation-bound egress. Add host-side Pi explanations from these facts. Production hardening includes private directory/file permissions, symlink-resistant writes, atomic publication and defined crash durability. Separate diagnostic-write failure from mandatory audit-write failure.
3. **Close demonstrated structured outcome gaps.** Stable reason/stage for launch/query/runtime/upstream failures, raw-exec metadata channel, and same-enforcement static checking. Do not gate consumer improvements on optional DNS probes or live subscriptions.

The spec must name new guarantees for retained per-run observations/provenance, independent bounded queries, availability/coverage, and optional exec metadata without stream alteration. Map each assertion to its new/existing guarantee; extend existing lifecycle/policy guarantees for bugs. Require macOS, Linux x64/arm64 and relevant full gates before shipping. Do not ship the scratch patch or treat format/Clippy as security validation.

Keep existing command forms and stdout schemas working initially. Add fields compatibly where consumers tolerate them; offer explicit schema selection for changed envelopes, reject unsupported major versions and tolerate unknown additive fields. Retain legacy stderr prose for humans without making it a machine contract. The proposed metadata FD is opt-in. Its generation binding, inherited descriptor ownership, missing/blocked/closed reader behavior and terminal management result need real runtime tests. A stale name must never silently target a replacement run: any future generation-conditional exec must bind dispatch to the intended runtime instance, not merely perform a racy list-then-exec check. That mechanism remains an implementation question. Keep reading `ready.egress_log` as an opaque path; callers hard-coding `<name>.jsonl` need migration. Existing name-only logs cannot be reliably assigned to a prior generation: label them legacy/ambiguous rather than silently importing them as complete run history. Release notes must list these caller changes. Do not silently change stdin ownership, teardown acknowledgements, audit failure, policy order, admission rules or runtime confinement.

## Unresolved evidence and decision triggers

- Retention limits, safe snapshot fields, permissions and crash durability need production design and tests. The bounded prototype is not a forensic audit store. A machine crash/SIGKILL can leave no terminal cause; fsync cannot create an event the process never observed.
- Full provenance and fine upstream reason capture remain unimplemented; source tracing establishes where to retain them, not correctness of a finished implementation.
- Runtime-independent query succeeded with no runtime executable. A blocked runtime/image-store experiment was not repeated here; the existing issue-89 evidence is historical, not this test. Optional probes still need real stall/cancellation validation.
- The TypeScript adapter handles tested finite output and orphan-held pipes. It does not establish arbitrary callback backpressure, exec cancellation or production protocol validation.
- Direct Pi interactive presentation was not tested with a human-driven TTY or model. RPC startup, host attach, current-config drift and retained-policy rendering were exercised.
- No comparator runtime was tested. No Linux arm64 prototype was run. No cross-platform full-suite/release claim is made.
- Reconsider an SDK after repeated same-language integrations demonstrate transport duplication. Reconsider a service only after a concrete remote/multi-client or client-independent lifetime requirement outweighs added authority and operations. Reconsider a core library only for a real embedding workflow that accepts signal/runtime/version coupling.

Independent review and final cleanup results follow below. Implementation remains a separate decision.

## Independent review, final results and cleanup

The [independent review](2026-10-15-diagnostics-evidence/independent-review.md) challenged twelve material issues. The report now distinguishes bounded reads from storage retention, requested from observed controls, process exit from EOF/teardown, counterfactual policy checks from observed requests, and runtime absence from runtime stalls. It explicitly discloses incomplete prototype provenance/terminal details, missing evaluator-version support, the untested metadata-FD transport, and the reconstructed baseline manifest. The reviewer endorsed the architecture direction after reading the report; no remaining material error or scope blocker was identified.

The final [Linux edge receipt](2026-10-15-diagnostics-evidence/linux-edge.json) resolves the observer experiment separately. With a real 4 KiB pipe and no reader, the query remained blocked after 200 ms. The owner still executed `printf still-alive`. SIGTERM ended only the observer (status -15, 4,081 captured bytes), and a fresh bounded query succeeded. Targeted runtime kill then produced `down/exited`, status 137, with retained history after teardown. The permitted `https://api.github.com/` control returned HTTP 200 on both tested runtimes. This establishes isolation of this finite diagnostic query from this blocked observer; it does not establish arbitrary SDK callback backpressure or all OS-stall behavior.

Resource ledger and cleanup:

| Owned resource | Disposition and verification |
|---|---|
| Ordinary source copy, reconstructed copy, target output, Zig/uv caches, three research binaries | Removed after preserving patch, hashes, build/check logs and small drivers. No managed Git worktree was created. |
| Two temporary Pi project repositories, private XDG roots, extracted init/profile/Pi artifacts, HTTP fixture sockets/threads | Removed with the owned local scratch tree; fixture servers were shut down and child processes reaped. |
| `pd-baseline-*`, `pd-prototype-*`, `pd-consumer*`, `pd-edge`, and the two Pi owners on macOS | No owned boxes remain. Every observed orphan was removed with exact-name `box down`. Owned name locks are unheld and ownership/control records absent. |
| `pd-linux-baseline*`, `pd-linux-prototype-*`, `pd-edge` on Linux | Native inventory empty; no research processes/mounts. `/tmp/pd-1015` removed, checked absent. Name locks unheld, no ownership/control records. |
| Comparator source checkouts and downloaded Docker SDK artifact/metadata | Removed after preserving source URLs/revisions and research notes. |
| Images, shared builder, shared services and configuration | No image created/deleted, no service restarted, no shared config changed. Existing builder and unrelated concurrent macOS E2E box retained. |
| Stable host coordination lock files | Intentionally retained, zero-byte and unheld, as the binding ownership contract requires: persistent lock files must never be unlinked. These are not live boxes or leftover owner records. |

Local ownership verification, Linux verification, Linux lock verification and the final cleanup receipt recorded the checks (receipts not retained). No broad prune or cleanup command was used. Only this dated report and its small reproducible evidence bundle were added to the requested checkout. The original tracked files still match the reconstructed baseline.

The decision is ready: authorize the staged subprocess-contract work, or reject/adjust the proposed guarantees and retention limits. A production SDK or service remains deferred unless a concrete consumer supplies evidence for it.
