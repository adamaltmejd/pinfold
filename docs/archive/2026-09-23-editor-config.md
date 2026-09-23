# Absent editor config, 2026-09-23

Found while filing the v1 backlog. The spec made `.vscode/`, `.claude/` and
`.idea/` read-only only when present. In a project without `.vscode/`, the
box could create `.vscode/tasks.json` on the host, and the host runs it when
the project is opened.

Chosen: every protected directory is mounted read-only whether or not it
exists. An absent one is created empty on the host first and removed after
the run if still empty. Inferred, not tested: a mount point inside the
project mount appears on the host either way, on both runtimes, so creating
it explicitly costs nothing extra and makes the cleanup pinfold's.

Rejected: leaving it to users to create the directories, and an overlay
over the whole project to hide new paths.
