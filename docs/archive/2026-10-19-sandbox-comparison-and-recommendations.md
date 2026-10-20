# Pinfold sandbox comparison and recommendations

Saved: 2026-10-19. Investigation and discussion: 2026-10-14 through 2026-10-16.

Pinfold's most useful improvements are safe access to existing Git worktrees, clearer diagnostics, browser access to boxed development servers where that workflow is needed, and containment of writable disk consumption. The comparisons do not establish a need for a public SDK, a persistent service, or a broader interactive agent frontend.

Keep Pi as the interactive focus. Keep security and policy enforcement in the binary. Improve the public subprocess contract before adding integration layers. Preserve worktree refusal until a candidate meets the actual filesystem and workflow requirements. Defer method/path authorization until a concrete need cannot be met by upstream credential scopes.

This report consolidates the feature comparison, the Linux VM clarification, the user's corrections, the diagnostics/API discussion, the SPACE comparison, and both research prompts. It records recommendations, not an approved change to the binding architecture.

## Evidence and dates

The October 14–16 comparison used Pinfold's working tree, source, historical investigations, and official competitor documentation and source. The Pinfold baseline was `198600b1751d889d354ed3657d8f839e8436e469` on `codex/issue-89-bounded-listing`, with relevant uncommitted changes. It was not a released-binary comparison. No competitor was deployed, and no comparative security or performance benchmark was run in this chat.

On October 19 the checkout was at `d8dc84c202921d11118f29d2245cccf61190c310`, described by Git as a local WIP snapshot. Saving this report did not rerun the competitor investigation. Competitor claims below retain their October 14–16 observation dates; links to upstream `main` and online documentation can subsequently change.

Separate October 15 worktree and diagnostics research produced disposable runtime experiments. Those reports are linked below and retain their own source baselines, methods and limitations. Their prototype findings are not production features or tests performed by this comparison.

Binding project references: [ARCHITECTURE.md](../../docs/ARCHITECTURE.md), [README.md](../../README.md), and [AGENTS.md](../../AGENTS.md). Archive reports are historical evidence, not specifications.

## Pinfold capabilities already present in the reviewed baseline

Pinfold already provided much of the basic machinery offered by competitors:

- Apple Container micro-VMs on macOS and rootless Podman containers on Linux. Linux boxes shared the host kernel; macOS boxes had separate guest kernels.
- A default-deny egress proxy, hostname allowlisting and SNI checks, explicit guest-to-host routes, and host-held credential injection. Claude and Codex login routes already existed.
- CPU and RAM limits, PID restrictions, resource observations, pinned harness artifacts, profiles, project homes and cleanup.
- A public JSON subprocess interface for lifecycle, inventory and statistics. `box exec` preserved command streams and exit status.
- Effective-configuration inspection, `doctor`, artifact inspection and JSON egress logs.
- A separate Pi Durable prototype for controller recovery. It recovered using host checkpoints and a fresh box, not by restoring VM memory or automatically replaying interrupted shell actions.

The principal gaps were therefore workflow support, retention and presentation of diagnostic facts, and particular containment guarantees. Reimplementing existing capabilities under new names would add little.

## Comparison with other systems

The systems have different deployment and trust models. A larger feature list is not evidence of stronger overall security or a better fit for a local Pi workflow.

| System | Relevant documented capabilities | Implication for Pinfold |
|---|---|---|
| Docker Sandboxes | Convenient agent launch, localhost port publishing, private Git clones, persistent environments and network-policy inspection. | Strong reference for everyday usability, browser previews, and policy explanation. Private cloning does not satisfy editing an existing linked worktree in place. [Usage](https://docs.docker.com/ai/sandboxes/usage/), [policy tools](https://docs.docker.com/reference/cli/sbx/policy/). |
| Matchlock | Linux and macOS microVMs, disposable workspace overlays, forwarding, SDKs and host-held secrets. The reviewed project described itself as experimental. | Useful mechanisms to study for workspace isolation and forwarding. Its Linux VM boundary has additional deployment requirements. [Repository](https://github.com/jingkaihe/matchlock). |
| Anthropic sandbox-runtime and Claude sandboxing | Native process confinement, filesystem/network policy and violation reporting. Claude's built-in sandbox applies to shell commands; wrapping a whole process with sandbox-runtime has a different scope. | Useful diagnostic examples. Built-in shell sandboxing is not equivalent to containing the complete agent in a Pinfold box. [Runtime](https://github.com/anthropics/sandbox-runtime), [Claude scope](https://code.claude.com/docs/en/sandboxing). |
| NVIDIA OpenShell | Credential isolation and configurable restrictions over HTTP methods/paths, GraphQL operations and MCP tool names. Audit mode records violations without enforcing those request restrictions. | Demonstrates the difference between hiding a credential and restricting its use. Additional authorization semantics need a demonstrated Pinfold requirement. [Network rules](https://docs.nvidia.com/openshell/latest/how-it-works/policies/network-rules). |
| E2B | Persistence and pause/resume capabilities in a programmatic sandbox platform. | Relevant when a controller needs retained execution state. This does not establish that Pinfold needs VM snapshots or an SDK. [Persistence](https://docs.e2b.dev/sandbox/persistence). |
| Daytona | Persistence, lifecycle management and previews. | Browser access is a relevant workflow. The larger platform lifecycle is a separate architectural choice. Capabilities and persistence guarantees must be checked for the selected runtime. [Persistence](https://www.daytona.io/docs/en/persistence/), [previews](https://www.daytona.io/docs/en/preview/). |
| Perplexity SPACE | Firecracker environments, an HTTP/OpenAPI interface, command-event streams, structured errors, service ingress and pause/suspend/restore. | Especially useful for diagnostic semantics and explicit caller contracts. Its service deployment and persistent lifecycle address broader needs than Pinfold's local process ownership. [Repository](https://github.com/perplexityai/SPACE), [architecture](https://github.com/perplexityai/SPACE/blob/main/docs/architecture.md). |

## Feature recommendations and user decisions

### Safe existing worktrees

This was the highest-value demonstrated blocked workflow. The requirement is to edit a complete existing linked worktree, including root manifests and arbitrary source directories, on the existing filesystem. Preserve dirty, staged and untracked work and useful host editing. Avoid copying the checkout or Git history on every launch. Commits remain host-owned.

The user correctly challenged presenting this as an obvious feature to add: it was already wanted, and no satisfactory implementation had been established. A private clone, restricted subdirectory mount or export/import workflow is a possible compromise, but does not meet the original requirement without qualification.

The critical question is which component actually handles writes to the host filesystem. Earlier experiments showed that literal `.git` pathname protection could be bypassed through case aliases or hardlinks. Applying Seatbelt to the VM-owning process did not constrain writes through its writable Virtualization.framework share. Removing guest Git or setting `GIT_DIR` does not protect an exposed writable host gitfile. Startup alias admission is a bounded admission check, not continuous filesystem enforcement.

These results reject particular mechanisms, not every alternative filesystem exporter or isolated-copy design. The recommendation was to commission disposable experiments and independent adversarial review, allowing either a proven candidate or a justified decision to retain refusal. The complete prompt is preserved in Appendix A.

Evidence: [worktree security and startup](2026-10-09-worktree-security-and-startup.md), [Switchyard requirement and Seatbelt](2026-10-09-switchyard-worktrees-and-seatbelt.md), [VM-share probe](2026-10-09-seatbelt-vm-share-probe.md), and [external hooks-path alias finding](2026-10-14-hook-path-alias-ticket.md).

### Diagnostics

This was the strongest small improvement and received explicit user support. The initial proposal was a readable summary of recent egress denials, with reasons and the source of the applicable configuration. The later API discussion established that presentation alone cannot recover facts discarded before logging or publication.

A useful implementation must distinguish a local policy refusal from DNS, connection, TLS, authentication and upstream failures where those facts are observable. A policy checker should share enforcement logic. It must distinguish static policy permission, DNS-dependent checks and observed network outcomes. An allowed decision is not proof of successful delivery. An encrypted CONNECT tunnel does not expose arbitrary HTTP paths or establish which agent command caused a request.

Keep changes explicit. Observing a denied hostname is not a reason to automatically allow it. A checker must not duplicate policy in an SDK or quietly widen it.

### Browser previews

The missing workflow is to start a development server inside a box and inspect it in the host browser. Existing Pinfold routes connect the guest to a host service; they do not provide the reverse direction.

A focused proposal would forward an explicitly selected host-loopback port to one service in a named box, with cleanup tied to its lifetime. WebSocket support matters for hot reload. The design must preserve the absence of general guest access to the host. Unsupported WebSocket upgrades on current routed HTTP services should not be confused with all WebSocket traffic through permitted TLS tunnels.

The recommendation remains conditional: prioritize this when web development is a regular use case. Docker, Matchlock, Daytona and SPACE provide relevant precedents, but their transport and authorization designs are not automatically suitable for Pinfold.

### Disk containment

CPU and memory limits do not stop a build or agent from filling storage. The reviewed architecture explicitly had no disk cap on either runtime. Monitoring and cleanup improve operations but do not establish a hard bound.

Any proposed guarantee must name its coverage: guest scratch storage, project home, project checkout, logs, caches, images and other writable host exports. Limiting a guest disk while leaving host mounts unrestricted would not bound total writable consumption. Investigate this for unattended workloads, as the user supported, before promising a universal limit.

SPACE also provides a separate question for research: limits on disk operations and bandwidth. Those address I/O pressure, not storage capacity. They do not establish aggregate fairness across many VMs on one host. [SPACE disk I/O](https://github.com/perplexityai/SPACE/blob/main/docs/space/design/guest-io.md).

### Credential use and route restrictions

Host-held injection protects the secret value, but the agent can still exercise the credential's authority against an allowed origin. Upstream credential scopes should be the first tool for narrowing that authority.

The initial comparison suggested selected methods and paths on injecting routes as a possible security improvement. The user judged this to be complexity without a demonstrated need. The resulting recommendation is to defer it. A future proposal must identify the operation that needs restriction and explain why upstream scopes are insufficient. HTTP method alone is not a complete authorization model; a GET is not universally harmless.

### Interactive agent scope

The original suggestion to provide equally convenient interactive launch for Pi, Claude and Codex was withdrawn after user feedback. Pi remains the interactive focus. Existing core harness and login support does not justify a broader frontend.

### Features to defer

Do not add full VM snapshots, a scheduler, several SDKs, an enterprise policy system, a generic secret manager or a dashboard merely for competitor parity. Revisit a feature when a concrete workflow or guarantee requires it. Linux microVM isolation is a legitimate separate design investigation if protection against guest kernel exploits becomes a requirement.

## Linux microVM clarification

The original Matchlock comparison omitted deployment constraints. Firecracker uses KVM to run a separate Linux guest kernel and requires read/write access to `/dev/kvm`. A container image can supply a VM's guest filesystem; the image format does not determine whether execution shares the host kernel. [Firecracker requirements](https://github.com/firecracker-microvm/firecracker/blob/main/docs/getting-started.md#kvm).

Matchlock's reviewed Linux setup granted `CAP_NET_ADMIN` and `CAP_NET_RAW` to its executable, configured TUN access, enabled IPv4 forwarding and added device-access group membership. Later commands could run without sudo, but the executable retained the granted capabilities. This differs materially from assuming an unprivileged drop-in replacement for Pinfold's current Linux deployment. [Setup implementation](https://github.com/jingkaihe/matchlock/blob/main/cmd/matchlock/setup_linux.go).

On an administered Linux host, this is a technical option. Within another VM, the outer hypervisor must expose nested virtualization. Within LXC, device and permission delegation must be established; LXC is not itself another VM, so that situation is not inherently nested virtualization. Compatibility with the actual Yard/LXC environment was not tested. [KVM nesting](https://docs.kernel.org/virt/kvm/x86/running-nested-guests.html).

Adoption would require guest-kernel, filesystem-sharing, network and lifecycle integration. A VM boundary alone also does not protect host Git metadata exposed through a writable share.

SPACE's documented local installation is heavier still: Linux x86_64/KVM, TUN, loop devices, btrfs and a Docker daemon permitting privileged containers, with administrative setup. The guide uses Kind and does not run directly on macOS. SPACE also documents a direct worker adapter, so Kubernetes should not be described as mandatory for every deployment. [Installation](https://github.com/perplexityai/SPACE/blob/main/docs/installation.md), [adapters](https://github.com/perplexityai/SPACE/blob/main/docs/architecture.md#request-and-execution-flow).

## Diagnostics and the public API

Pinfold already has an API: its subprocess protocol. `box up`, `exec`, `stat`, `list`, `down` and `prune` are programmatic operations. The first-principles question is whether their data and failure semantics let a consumer integrate correctly. Choosing HTTP, a library or an SDK does not by itself preserve missing evidence.

Four problem classes help assign responsibility:

| Problem | Example from the investigation | Where to address it |
|---|---|---|
| Fact never retained | Configuration merging loses the winning source; some upstream failures lose their cause. | The binary that observes or resolves the fact. |
| Inconsistent public representation | Startup refusals contain reason/detail; some later failures contain detail only; listing errors use stderr. | The public contract and serializers. |
| Consumer loses available information | Pi Durable drains stderr without retaining it, drops startup detail, and stops reading lifecycle events after readiness. | The controller adapter. |
| Information is difficult to understand | Users must locate and interpret raw egress records. | Human CLI or Pi presentation over structured facts. |

The Pi Durable observations came from source review. Its current no-egress setup does not establish network-diagnostics requirements. An egress-enabled consumer is needed for those experiments. Relevant implementation: [prototype.ts](../../examples/pi-durable/prototype.ts), [CLI](../../crates/pinfold/src/cli.rs), [configuration](../../crates/pinfold/src/config.rs), and [proxy](../../crates/pinfold/src/core/proxy.rs).

The proposed direction is binary-owned structured diagnostic facts, consumed both by human presentation and programmatic callers. A thin SDK is an option if repeated integration code or lifecycle mistakes demonstrate that it reduces total complexity. It should not reproduce policy, introduce independent security authority or force a persistent daemon.

The important contract questions are:

- Identify the particular box generation, including after a name is reused.
- Preserve the applied configuration and its provenance. Reading files later describes what a new launch might use, not necessarily what an existing box used.
- Distinguish absent, unsupported, unavailable, stale and incomplete evidence. Do not infer success from an empty result.
- Preserve facts from startup through teardown where justified; do not invent a terminal cause after an unobserved owner death.
- Keep historical queries and observations bounded. A stalled runtime must not prevent access to already retained host facts.
- Preserve raw command streams and exit status. A uniform JSON wrapper must not corrupt `exec` I/O.
- Preserve ownership: `box up` owns the box; closing its stdin ends that lifetime. A diagnostic observer must not acquire teardown authority or change fail-closed behavior.
- Keep secret values out of logs and artifacts. Do not expand traffic capture casually.
- Explain compatibility and caller changes before adopting a new contract.

A successful investigation should compare consumer repairs, focused subprocess additions, a coherent subprocess API with an optional thin SDK, and broader library/service alternatives where justified. It should finish with working examples and decisive experiments, not only an interface diagram. Appendix B preserves the full goal prompt.

## SPACE findings relevant to Pinfold

### Error and recovery semantics

SPACE centralizes public error codes, their status mapping and retryability, with request IDs and details. Its implementation can clear retryability after work has committed. Its troubleshooting guidance distinguishes a client wait deadline from a process deadline and from an operation whose result is unknown. This is useful material for Pinfold's error model: callers need to know whether work was rejected, completed, or may still be in progress. A retryable label alone is insufficient unless endpoint deduplication and known effects support it. [Error implementation](https://github.com/perplexityai/SPACE/blob/main/crates/libs/api/src/routes/public/error.rs), [troubleshooting](https://github.com/perplexityai/SPACE/blob/main/docs/troubleshooting.md).

### Bounded diagnostic evidence

SPACE gathers layer-specific failure evidence, including resource/OOM context and guest connectivity, under a deadline. It marks incomplete evidence as partial and bounds diagnostic work. The transferable principle is a useful, limited failure record correlated with the relevant execution. It does not require copying a continuous watchdog or telemetry service into Pinfold. [Diagnostic collector](https://github.com/perplexityai/SPACE/blob/main/crates/libs/sbx/mgr/server/src/watchdog/diagnostics.rs), [VM diagnostics](https://github.com/perplexityai/SPACE/blob/main/crates/libs/sbx/mgr/server/src/vm_diagnostics.rs).

### Command output is not durable history

SPACE separates an accepted lifecycle operation, a running sandbox and a successful command. Creating a command session does not itself start the process; attachment starts it. A successful stream requires a terminal process result, not just an HTTP response or a closed connection.

Its native command output is live rather than durable replay. Reconnection with `Last-Event-ID` reports `resync_required`; running-process inspection does not provide completed-command history. This is an important limitation to preserve in comparisons. A richer API does not automatically make lost output recoverable. Pinfold should likewise describe incomplete observations honestly if it ever adds detached execution or reattachment. [First sandbox](https://github.com/perplexityai/SPACE/blob/main/docs/first-sandbox.md).

### SDKs and telemetry

SPACE's native walkthrough uses Python's standard library without an SDK. Its E2B compatibility wrapper serves existing E2B clients and has a specific tested Python baseline; the reviewed documentation did not establish equivalent JavaScript SDK coverage. It is not an unconditional drop-in implementation of every E2B feature. [Walkthrough](https://github.com/perplexityai/SPACE/blob/main/docs/first-sandbox.md), [compatibility limits](https://github.com/perplexityai/SPACE/blob/main/docs/ecw.md).

SPACE also provides metrics and traces through Datadog or OpenTelemetry configurations. Exported telemetry is not itself a caller diagnostics API. Pinfold should first expose useful local facts rather than require a telemetry backend. [Observability](https://github.com/perplexityai/SPACE/blob/main/docs/observability.md).

### Lifecycle and browser features

SPACE supports resident pause and snapshot-based suspension that releases active VM resources, followed by wake/restore. Its environments can outlive client requests and command streams. Service ingress supports HTTP and WebSockets. These are substantial features, but adopting their lifecycle would change Pinfold's process-owned disposable-box model. Browser preview can be investigated separately without importing snapshot orchestration. [Architecture](https://github.com/perplexityai/SPACE/blob/main/docs/architecture.md).

The reviewed versioning document still planned breaking changes before a stable 1.0 contract. Treat documented capabilities, compatibility promises, source implementation and verified runtime behavior as distinct evidence. [Versioning](https://github.com/perplexityai/SPACE/blob/main/docs/versioning.md).

## Related research available by the SPACE comparison

The October 15 [diagnostics architecture report](2026-10-15-diagnostics-architecture.md) recommends retaining the process boundary, repairing consumers, then adding generation-specific retained facts, provenance, structured outcomes and bounded reads. It explicitly defers a public SDK, embedded core library, multiplexed RPC server and persistent service. Its Mac and Linux prototype observations have stated limitations; the report is not a landed implementation. SPACE strengthens the rationale for the public-contract work without overturning that recommendation.

The October 15 [native worktree protection report](2026-10-15-native-worktree-protection.md) identifies a narrower candidate using native Apple sharing, all eight ASCII `.git` case masks, genuinely read-only external metadata and existing alias admission. Its selected native probes passed on the tested APFS/runtime combination. That supersedes presenting the earlier failed literal-path mechanism as the only available direction. It does not establish universal alias protection, unrestricted topology changes, or a passing integrated Pinfold guarantee suite. The report's constraints and proposed next experiments govern how the candidate should be evaluated.

These later reports should be read before starting another research session. The prompts below are preserved as issued, not instructions to repeat completed experiments.

## Recommended next decisions

| Decision | Recommendation | Evidence required before adoption |
|---|---|---|
| Diagnostic contract | Prioritize preserving useful facts and repairing consumer information loss. | Real CLI/runtime failures, successful controls, complete lifecycle handling, bounded unavailable/partial results, secret-safe output. |
| Worktree support | Continue with the strongest evidence-backed candidate while preserving refusal outside proven conditions. | Host-observed metadata integrity, alias/race tests, complete-worktree usability, explicit supported filesystem/runtime assumptions and integrated guarantee coverage. |
| Browser previews | Advance when the regular web-development workflow warrants it. | Loopback-only access, named service scope, WebSocket/hot-reload behavior, lifetime cleanup and preserved network isolation. |
| Disk containment | Perform a focused feasibility investigation for unattended use. | Coverage of each writable storage class, actual hard-limit behavior and failure handling. Assess I/O-rate control separately. |
| SDK or persistent service | Defer unless a repaired contract leaves demonstrated integration burden. | Comparison of direct clients and a thin library, including lifetime, cancellation and failure behavior; maintenance cost must decrease. |
| Route authorization | Defer until upstream scopes cannot meet a named requirement. | A specific prohibited operation, service-specific authorization semantics and enforcement tests. |
| Broader interactive agents | Keep Pi as the focus. | A new user requirement, rather than existing core support alone. |

No implementation, release, merge, infrastructure change or new research session is authorized merely by this report. Each research prompt below includes its own experimental scope and cleanup requirements.

## Appendix A Original worktree research prompt

Issued October 15. This historical prompt explicitly allowed a conclusion that no candidate yet met the requirements. Select the intended research model in a new session if reusing it, and first account for the later worktree report above.

```text
Create a research goal: determine the best practical way for Pinfold to support safe Git worktree workflows, and present an evidence-backed recommendation for my approval.

Work in /Users/adam/Code/pinfold. This is research with disposable implementation experiments. Complete the investigation and recommendation; do not land a production implementation, change the binding contract, or enable worktrees in my installed Pinfold.

Desired workflow

Contained agents should edit complete existing linked Git worktrees, including root manifests and arbitrary source directories, on my existing case-insensitive Mac filesystem. Preserve existing dirty, staged and untracked work and useful interaction with host editing. Avoid copying the checkout or Git history on every launch. Linux support also matters.

Host Git metadata and protected host-executed configuration must remain protected. Commits remain host/caller-owned. Keep Pi as the interactive focus and keep Switchyard responsible for orchestration.

These describe the preferred outcome. If no sound design meets all of them, identify the smallest necessary compromise and measure its cost. You may recommend an existing component/tool, a different filesystem-sharing design, an isolated working-copy workflow, or an explicit architecture change. Do not silently redefine the requirements to make a candidate pass.

Starting evidence

Read AGENTS.md and the relevant current ARCHITECTURE.md sections. Inspect the actual checkout, branch, changes and runtime versions. Preserve all unrelated work. Record the exact source baseline used for experiments; a clean checkout may omit relevant uncommitted changes.

Read these historical reports in order, following their evidence references where needed:

/Users/adam/Code/pinfold/docs/archive/2026-10-09-worktree-security-and-startup.md
/Users/adam/Code/pinfold/docs/archive/2026-10-09-switchyard-worktrees-and-seatbelt.md
/Users/adam/Code/pinfold/docs/archive/2026-10-09-seatbelt-vm-share-probe.md

Also inspect the newer hook-alias finding and verify its current implementation status:

/Users/adam/Code/pinfold/docs/archive/2026-10-14-hook-path-alias-ticket.md
/Users/adam/Code/pinfold/docs/archive/2026-10-14-measured-build-recommendations.md

Important established findings:
- Literal .git protection was bypassed through case aliases and hardlinks.
- Applying Seatbelt to the VM-owning process did not constrain writes through its writable Virtualization.framework share.
- Removing guest Git, hiding common metadata or setting GIT_DIR does not protect an exposed writable host gitfile.
- Startup alias admission is not continuous filesystem enforcement.
- Scoped subdirectory editing does not satisfy the complete-worktree requirement.
- These findings do not disprove every custom filesystem exporter, isolated-copy design or alternative runtime.

Do not repeat a failed approach unless you identify a materially different mechanism or changed upstream behavior.

Research and experiments

Review current primary documentation and relevant source from other tools. Starting points include Matchlock, Docker Sandboxes, Anthropic sandbox-runtime, agent-sandbox.nix, Agent Safehouse, devsandbox, NVX and git-lazy-mount. Follow the promising mechanisms rather than exhaustively reviewing every product.

For each serious candidate, establish:
- Which component actually handles host filesystem writes.
- Whether it protects host metadata, substitutes private metadata, copies/synchronizes files, or permits Git writes.
- Its dependencies, privileges, platform requirements and operational burden.
- Which Pinfold guarantees and workflow properties it preserves or changes.

Compare genuinely distinct approaches. Consider current-runtime improvements, enforcement in a filesystem service, and isolated working files with controlled application of changes. Consider storage or runtime changes where justified. Prefer existing capabilities and components over building a general filesystem or synchronization subsystem.

Run the smallest decisive experiments first. Reject candidates when an essential property fails, and continue with the strongest remaining alternatives. Do not stop at a literature survey or the first happy-path demonstration.

For promising candidates, use real runtimes and disposable repositories. Test ordinary root-level edits and intended Git reads alongside adversarial cases. Cover the relevant combinations of:
- .git pointer modification, replacement, unlink and rename;
- case aliases, existing/new hardlinks, symlinks and overlapping exports;
- common/per-worktree metadata, commondir, config and effective hooks paths;
- malformed layouts and discovery-to-use races;
- concurrent boxes and ordinary host changes.

Unsupported layouts may be refused explicitly. Define concurrency and trusted-host assumptions rather than quietly assuming a frozen filesystem.

Observe host bytes, file identities and harmless execution markers after real host Git operations. A guest command returning failure is insufficient by itself. Distinguish source inspection, native-process probes, framework-level VM probes and Pinfold integration. Report macOS and Linux evidence separately.

For copy/overlay/import approaches, investigate dirty-state preservation, lower-layer stability, conflict handling, protected-path rejection, symlink/hardlink redirection, crash recovery and whether host edits can be lost.

Measure preparation, readiness, useful work, applying changes and teardown separately on representative repositories. Report cold/warm conditions and storage costs. Do not infer practical performance from a tiny fixture.

Use subagents for bounded research and independent adversarial review. Serialize shared-runtime experiments; do not run competing Apple Container/BuildKit suites. Have an independent reviewer challenge the leading recommendation and its evidence before concluding.

Authority and cleanup

You may create temporary checkouts, prototypes, synthetic repositories, local builds, boxes, images and mounts needed for this research without asking at every step. Keep experimental changes isolated. Use the homelab-trial-vm skill if a disposable Linux host is needed.

Do not modify live Yard services, global runtime/security configuration, existing repositories or shared infrastructure to make an experiment work. Privileged installations or host changes belong in an authorized disposable environment; ask only if a necessary experiment cannot be isolated.

Track resources you create. Clean up your processes, boxes, mounts, images, temporary checkouts and build artifacts, verifying ownership before removal. Never use broad prune/reset operations or stop unrelated workloads.

Retain a compact dated Markdown report, small runnable reproduction scripts, relevant patches and essential results under docs/archive. Remove bulky generated artifacts. Verify cleanup and report any leftovers honestly.

Deliverable and stopping point

Present:
1. One recommended direction and why it best fits the actual workflow.
2. A comparison of the serious alternatives, including rejection reasons.
3. Reproducible evidence, exact versions and measured costs.
4. Security assumptions, remaining uncertainty and untested cases.
5. The smallest proposed production change and its required guarantees/tests.
6. A verified cleanup summary.

Do not present prototype success as a complete security proof. If no candidate is suitable, explain the concrete blocker, the least costly compromise, and what new evidence or upstream capability would justify revisiting it.

Finish when the evidence supports a useful decision and independent review has been addressed. Present the suggestion to me before adopting it.
```

## Appendix B Original diagnostics research prompt

Issued October 15. The subsequent diagnostics report above addresses this research goal. Preserve its distinction between experiments and production implementation if reusing the prompt.

```text
Create and pursue a research goal: determine the best architecture for
Pinfold diagnostics and their integration into Pi and external host
controllers. Investigate whether this calls for improving the existing
process interface, a more deliberate public API, an SDK, or a broader
change. Present an evidence-backed recommendation for my decision.

This is research with disposable prototypes. Do not ship a production
implementation, merge changes, or change the current security contract.

Work in /Users/adam/Code/pinfold.

Context and objective

I want humans and integrations to answer practical questions reliably:
- Why did a launch or operation fail?
- Why was a network request refused?
- Which configuration source supplied the relevant setting?
- What policy was actually applied to this particular box run?
- What happened before the box stopped, including unexpected termination?
- Is information unavailable, unsupported, stale, or genuinely absent?

Interactive work remains focused on Pi. External controllers also matter.
Diagnostics should be easy to integrate without parsing human prose,
duplicating policy logic, or depending on undocumented internal files.

Start from these workflows and the current implementation. Do not assume
that adding a diagnostics command is sufficient, or that an SDK or server
is necessary.

Establish the baseline

Read AGENTS.md and the binding docs/ARCHITECTURE.md. Record the current
commit and relevant uncommitted changes. Preserve all existing work.
Treat docs/archive as historical evidence, not the current contract.

Trace the current public interfaces and diagnostic information through:
- box lifecycle, execution, listing and resource statistics;
- proxy decisions and egress logs;
- configuration merging, provenance and trust;
- doctor and runtime failures;
- Pi and examples/pi-durable.

Inspect another real controller if its source is available. Do not invent
requirements from an unavailable integration.

Separate four kinds of problem:
1. The binary never observes or records the necessary fact.
2. The public interface represents it inconsistently or incompletely.
3. The consumer receives it but loses it.
4. The information exists but is hard for a human to understand.

Current source suggests Pi Durable discards subprocess stderr and startup
detail, and stops reading lifecycle events after readiness. Verify those
observations and use them as concrete integration cases. Its current
no-egress setup does not establish network-diagnostics requirements.

Research alternatives

Review a small, relevant selection of other tools using current official
documentation and source. Consider Docker Sandboxes, Matchlock, and SDK-
oriented tools such as E2B or Daytona. Follow relevant alternatives you
discover. Compare actual diagnostic and integration behavior, not feature
lists, and distinguish documented claims from tested behavior.

Evaluate alternatives including:
- repairing consumers and making focused additions to the existing API;
- a coherent subprocess API with structured results, errors and events;
- a thin SDK over that interface;
- a library interface or persistent service where demonstrated needs
  justify the additional architecture.

Be willing to recommend a broader redesign if it reduces total complexity
or materially improves correctness. Equally, conclude that no SDK is
needed if direct integration remains straightforward.

Identify what belongs in the binary, the public contract, presentation,
and any client library. Security and policy enforcement remain in the
binary. An SDK must not become an independent implementation of them.

Investigate the difficult semantics

Determine the minimum necessary contract for:
- stable machine-readable reasons and useful human explanations;
- box identity across teardown and reuse of the same name;
- applied policy versus configuration read later, including provenance;
- diagnostic access during startup, execution and after termination;
- bounded queries, retention and live observation where workflows need it;
- unavailable or partial results and differences between runtimes;
- cancellation, deadlines and slow or disconnected consumers.

Preserve raw command stdout/stderr and exit behavior. Examine ownership
carefully: box up owns the box, and closing its stdin ends that lifetime.
An observer or SDK must not silently change ownership, teardown, or
fail-closed behavior.

Explore a policy explanation/checking operation using the same rules as
enforcement. Specify what it can establish without making a request.
Distinguish static policy decisions, DNS-dependent checks and observed
network outcomes. An allowed decision does not prove successful delivery.
Do not promise request-path or command attribution the proxy cannot see.

Keep secret values out of diagnostics, logs and saved artifacts. Do not
expand traffic capture casually. Do not add automatic allowlisting,
method/path authorization, or a generic policy language without a
demonstrated need.

Prototype and test

Build disposable, end-to-end prototypes of the strongest alternatives.
Use the real binary and runtime, consistent with repository test rules.
Exercise both a human-facing Pi diagnostic workflow and a programmatic
consumer. Compare the integration code and failure handling each requires.

Include successful controls and deliberately induced failures. Select
cases that discriminate between designs: denial versus upstream failure,
configuration changed after launch, startup refusal, unexpected teardown,
box-name reuse, and observer cancellation or interrupted output.

Test bounded behavior and cleanup. Identify which facts survive failure
and which cannot be recovered. Avoid making diagnostics depend on the
same stalled subsystem they are meant to explain.

Use available authorized macOS and Linux environments where practical.
State exactly what was tested and what remains unverified. Do not present
source review or a local fixture as proof on another runtime.

Use subagents for bounded research and independent review. Coordinate
builds and shared runtime experiments in the main session.

Autonomy and cleanup

Proceed autonomously with reversible experiments in clearly owned scratch
locations. Track every temporary repository, worktree, box, image, process,
mount and configuration change you create.

Do not modify unrelated dirty work, weaken controls, change shared
infrastructure, restart shared services, or perform broad pruning.
Remove your temporary resources and verify cleanup, including after failed
experiments. Preserve only the report and small reproducible evidence
needed to assess the recommendation.

Deliverable and completion

Write a dated research report in docs/archive and present the recommendation
to me. Include:
- the concrete workflows and demonstrated shortcomings;
- alternatives compared on correctness, usability, integration effort,
  maintenance, dependencies and operational cost;
- prototype results and limitations;
- the recommended public interface, illustrated with realistic human and
  programmatic examples;
- whether an SDK is warranted, for which consumer/language, and precisely
  what it should own;
- compatibility or migration implications for current callers;
- the smallest useful implementation scope and required spec changes;
- unresolved questions and what evidence would change the recommendation;
- cleanup verification.

Have an independent reviewer challenge the recommendation for unnecessary
abstraction, lost diagnostics, misleading explanations and changed
lifecycle/security semantics.

The goal is complete when the recommendation is concrete enough to approve
or reject, supported by experiments, and temporary resources are cleaned
up. Implementation of the recommendation is a separate decision.
```

## Appendix C SPACE addition to the diagnostics comparison

Suggested October 16:

> Review SPACE's structured error contract, operation/outcome distinctions, bounded partial diagnostics, and output-loss semantics. Identify which guarantees Pinfold can provide through its existing subprocess interface, and demonstrate any remaining need for an SDK.
