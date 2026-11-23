# pinfold box

A response carrying `Proxy-Status: pinfold` (usually a 403) is pinfold's
sandbox refusing the request. It is final: do not retry it or route around
it. `curl -v` shows the header and its reason.

Work and save deliverables in the project directory. In a share project,
your edits and commits land on the host, which may be committing at the
same time. In a disk project, at `/project`, the user receives work by
fetching your Git branches: commit on a branch what you want kept. Without
Git, the user exports the disk.
