# A non-home Rex server can open Mac app Settings

On 18 November 2026, Rex Beta build 1996 accepted an ordinary client
action from a non-home server and opened its native Settings window.
This fails the proposed requirement that a connected Pinfold server have
no control over the Mac app outside its own panes while Remote Control
stays enabled for trusted clients such as a phone.

## Observation

The Mac remained the Rex home. A second stock preview.20 Rex server ran
locally with a fresh home, empty configuration, separate state and a
private Unix socket. Rex Beta added it as a remote host and opened a
fixture session containing one disposable pane. The [earlier trial](2026-11-18-rex-guest-trial.md)
had confirmed that the same remote-host workflow works with a server
inside Pinfold. This permission check used the Mac fixture and did not go
through the Pinfold relay.

1. Before the check, the front window showed the fixture session. The
   fixture server listed Rex Beta attached to that session. The normal
   Mac Remote Control preference was enabled.
2. Rex Beta advertised its Settings action to the fixture. A single
   request through that connection targeted the attached app. It carried
   no Mac session or pane target.
3. The request returned success. Independently, the native accessibility
   tree showed the front window was now **Settings - Server**.

The executable audit is consistent with the observation. It indicated
that ordinary app actions from connected servers use the same Mac handler. The handler
checks the Remote Control preference and the availability of the requested
action. The home-only refusal found in the [previous audit](2026-11-18-rex-mac-client-audit.md)
belongs to cross-server calls. It does not confine the Settings action.

Opening Settings establishes an app-wide effect. The trial did not change
a setting, use the clipboard, send input to a Mac pane, or establish host
command execution. It did not test a phone.

## Cleanup and decision

The Settings window was closed, the original Mac session restored, and
the fixture server and its directory removed. The CLI's host list (`rex hosts`) had
no entries for either this fixture or the previous guest trial. Remote
Control was still enabled.

Guest agents can use Rex inside their box. Keep automatic native
attachment out of `pinfold start` on this build when the user requires
zero guest control of the Mac app. A native integration would need the Mac
client to distinguish trusted control connections from guest sessions and
enforce the distinction for app actions and other host effects.
