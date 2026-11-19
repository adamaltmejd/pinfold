# Rex in a project box on macOS

On 18 November 2026, Rex Beta build 1996 on the Mac connected to a Rex
preview.21 server in a running Apple project box. The host CLI was
preview.20, API revision 16. A host-only Unix socket relayed the connection
through Pinfold to a Unix socket in the guest. The Mac Rex server socket
stayed on the Mac.

A guest pane created a child pane. The host CLI read the guest terminal.
After the operator added the relay as a remote host in Rex Beta, the guest
server listed Rex Beta attached to the trial session. The operator typed
`native-check` and the guest terminal printed `Guest received [native-check]`.
The trial used a trusted guest.

Shutdown removed the relay, guest server and temporary files. The original
Mac session and guest Pi process were still running. One saved host entry
reappeared after Rex acknowledged its removal. Its socket was gone.

## Isolation finding

A separate guest server keeps ordinary guest session and pane operations
inside the box. Attaching a native client adds another path for requests.
Three parts of Rex's contract matter here.

- The installed CLI's `rex api describe client.perform` says the server
  passes session and block targets to the client without validating them.
  Rex documents Remote Control as an app setting and lists clipboard paste
  among Mac client actions. [Mac settings](https://www.superlogical.com/rex/docs/reference/mac-settings#setting-client-info-remote-control) · [Mac actions](https://www.superlogical.com/rex/docs/reference/mac-actions#action-pane-paste)
- `rex api describe client.action` and `rex api describe client.reply`
  describe a separate feature. An app may make other servers available to a
  running action. The server can then ask the app to make a call on one of
  them. Which servers the Mac app makes available in a guest session remains
  unverified. [Lua server scope](https://www.superlogical.com/rex/docs/reference/lua#hosts-and-other-servers)
- The server API also defines prompts sent to connected clients. A review
  of restricted connections would need to cover those and terminal events
  with host effects, including clipboard writes. [Server API](https://www.superlogical.com/rex/docs/reference/api)

We found no documented Mac-enforced restriction for an individual endpoint
covering these paths. The trial established native interaction. Whether a
hostile guest can cause actions on the Mac remains unverified.
