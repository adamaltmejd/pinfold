# Project files in pinfold

Use the project's actual current directory for work and deliverables.

For a share project, edits already affect host files. Stage and commit with
ordinary Git; a commit lands on the host's branch. The host may be editing
and committing at the same time.

For a disk project, commit work in the box. The user runs `pinfold fetch`
to bring those branches to the host. Uncommitted and home-only files do not
cross that way; the user can explicitly export retained disk files.

These instructions are guidance. Pinfold enforces the filesystem boundary.
