# Rex host effects with Remote Control off, 19 November 2026

Question for #336: with the Mac app's Remote Control preference off, which
paths still let a guest Rex server act on the Mac?

## Setup

Rex Beta build 1996 on the Mac, CLI preview.20. `rex.client.remoteControl`
read 0; the operator, the person at the Mac, had turned it off after the 18 November trials. This
project's box ran under `pinfold start` (profile full). The
profile home's Linux Rex server, the binary left from the 18 November
trial (version not re-checked), ran as `pinfold -- rex server run`. A host
relay listened on a 0700 directory and ran
`pinfold -- rex --autostart=false -S unix:///home/pi/.local/state/rex/run/server.sock __internal dial-stdio`
per connection (socat). The operator added the relay as a remote host in
the app and opened the guest session `audit`. The app then appeared as a
client of the guest server, attached to that session. A Mac-only HTTP
fixture listened on 127.0.0.1:8899 with a marker file.

Guest actions were sent with the host CLI against the relay. Mac state was
read from an unsandboxed Mac pane.

## Results

| Path | Guest action | Observed on the Mac |
|---|---|---|
| App actions | `client.perform` of `client.settings`, `client.update.check`, `client.host.forward_port` | Each refused: "Rex.app is not accepting actions. Turn on Remote Control". The same refusal came from the Mac's own server. |
| Clipboard write | OSC 52 set from a guest pane | First run, operator away and the window state unknown: Mac clipboard unchanged. Second run, operator at the Mac with the guest window in front: the Mac clipboard held the guest's text. No prompt. Which difference mattered is not isolated. |
| Clipboard read | OSC 52 query, raw tty, 4 s and 6 s | No reply bytes in either run. The second run had the guest window in front. No prompt. |
| File transfer | One OSC 1337 `File=` with `inline=0`, operator away | No new file in ~/Downloads. No other location was checked. |
| Listen tunnel | Guest created a `tunnel.listen` block on guest 127.0.0.1:9771 with `client_target` 127.0.0.1:8899 and placed it in the session | Before an owner: connections reset. About 20 minutes later the app was listed as the block's owner. The operator had not focused it. A guest connection then hung until the 5 s timeout. The Mac fixture logged no request. Whether the app opened any TCP connection is not known. |
| Dial tunnel | Guest created a `tunnel.dial` block with `client_address` 127.0.0.1:9772 | Nothing listened on Mac 127.0.0.1:9772, before or after the app owned the block. |
| Host list | Guest server `hosts.put` of a host `GuestPlanted` | Not in the Mac CLI's `rex hosts`, nor in the app's host switcher. |
| Notifications | OSC 9 and OSC 777, sent 6 s apart | Both shown as macOS notifications. |
| Links | OSC 8 hyperlink and a plain URL, both `http://127.0.0.1:8899/marker.txt` | The operator clicked both. One `fwd -L` block resulted; which click created it is not known. Without confirmation, the app created a dial tunnel block `fwd -L 127.0.0.1:63430:127.0.0.1:8899` in the guest session, listened on Mac 127.0.0.1:63430 (`lsof`: process Rex) and the operator reported the Mac browser opening `http://localhost:63430/marker.txt`. Nothing served it in the guest then. The Mac fixture on 8899 saw nothing. After a guest server answered on guest 127.0.0.1:8899, `curl` on the Mac at localhost:63430 returned its marker. The listener outlived the click. |

The app claimed ownership of guest-created tunnel blocks by itself. The
Mac fixture saw no request through them, and nothing listened on the dial
block's stated port. Only the operator's link click produced a Mac
listener, and it forwarded into the guest.

## Summary

With Remote Control off, the three app actions tried were refused, two
clipboard reads got no reply, one file transfer saved nothing in
~/Downloads, and guest-created tunnels produced no request to the Mac fixture and no
Mac listener. The guest raised macOS notifications. One of two clipboard
writes overwrote the Mac clipboard, with the guest window in front; why
the other did not is not isolated. Links it
printed opened the Mac browser on a click. An OSC 8 link's visible text
need not match its target. A click on a loopback link left a Mac loopback
listener into the guest, and the browser opened that listener's URL. A
later `curl` through the listener returned guest-served content.

Not tested with Remote Control off: server-initiated prompts, and calls to
other servers through `client.action` and `client.reply`, both open in the
[18 November audit](2026-11-18-rex-mac-client-audit.md).

## Side findings

- The CLI's `rex hosts` and the app's host switcher diverged. A host added
  with `rex hosts add` was not offered in the switcher until the operator
  added it in the app. The 18 November cleanup reported no hosts in the CLI
  list. On 19 November, before this trial, both the CLI list and the
  switcher showed that trial's host. The operator removed it in the app.
- `rex -S HOST hosts add` writes to the caller's home, not to HOST, as the
  CLI help states.
- The box proxy answered a plain HTTP request to a guest loopback port with
  an empty 403, the case #333 describes.
- Removing a host with `rex hosts remove` while the app was connected did
  not hold: the host the app had added reappeared in the CLI list within seconds.

## Teardown

The guest tunnel blocks, including the app's `fwd -L` block, and the
guest's planted host were removed. The relay, the guest server and the Mac
fixture were stopped, and `pinfold stop` exited 0. Nothing listened on Mac
127.0.0.1:63430 afterwards. The host the operator added in the app remained
for removal in the app.
