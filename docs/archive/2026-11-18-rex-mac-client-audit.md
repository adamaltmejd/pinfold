# Rex Mac client isolation audit, 18 November 2026

**Finding.** This review did not establish the isolation required to
automatically connect the user's native Rex app to an untrusted box.

## What was inspected

The Mac executable was the signed Superlogical Rex Beta build 1996, arm64,
source revision `d29bb605bec3eb79567b802aa921776615d4c4af`. Its SHA-256 was
`e68fcd65d51452378f7deb3097e39118921c31460df706c452a676835fac61c1`.
`codesign --verify --strict` succeeded. The installed CLI was preview.20.
The review used the shipped executable's control flow and Swift type
metadata, the local preference value, the installed API schemas, the native
app's advertised actions, and Pinfold source at `1b17449`. No matching Mac
source or debug symbols were found in the installation or in the vendor's
public release material. [Official release feed](https://blob.superlogical.com/public/rex-app/preview-arm64/appcast.xml)

The [guest trial](2026-11-18-rex-guest-trial.md) verified native input and
guest pane management. This follow-up did not reconnect the app to the
guest, exercise a request against a Mac pane, or access clipboard contents.

## Findings

**Calls to other servers.** The Mac executable contains a conditional path
that compares identities and returns the refusal “this app makes calls to
other servers only for its home server.” Its compiled model distinguishes
the home and focused servers. The local server reported that this Mac was
the configured home. The ordinary guest endpoint in the trial had been
added as a remote host. The branch exists in the inspected code, which the generic API
description alone did not establish. The static check did not exercise
how the app classifies the relay endpoint, other aliases or a reconnect.

**Mac app actions.** A separate Remote Control handler reads the boolean
preference `rex.client.remoteControl` and takes a refusal branch when it
is false. It also checks whether an action is offered and available in the
current context. The preference on this Mac was enabled. The handler's
compiled model includes a window registry, and the app models the
requesting server's scope. Those are indications of contextual routing.
They do not establish a per-guest authorization rule. The installed host
entry schema exposes no action-permission field. The Mac app advertised
clipboard paste, host switching and session selection to the local server.
The audit did not establish that the app offers or accepts those actions
from a non-home guest. [Mac settings](https://www.superlogical.com/rex/docs/reference/mac-settings#setting-client-info-remote-control) · [Mac actions](https://www.superlogical.com/rex/docs/reference/mac-actions)

**Clipboard and prompts.** The binary contains a clipboard write model
with grant and denial states. The protocol also has server-initiated
prompts. Finding a grant check in one path does not establish its coverage
across terminal events, native actions and prompts. These remain outside
the isolation verdict. [Server API](https://www.superlogical.com/rex/docs/reference/api)

**Relay lifetime.** The Apple runtime starts the box with networking
disabled. The trial carried native Rex traffic over a host-initiated exec.
Each new `pinfold -- …` invocation discovers the current project owner and
may start a box. An old relay accepting a new client could therefore select
or start a replacement. Pinfold already
checks the generation during admission and cancels an admitted exec on
lease loss. This finding comes from source review. The trial did not
exercise a restart. See `crates/pinfold/src/core/runtime/apple.rs` and
`crates/pinfold/src/pi/session.rs`.
