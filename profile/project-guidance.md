# Project files in pinfold

Use the project's actual current directory for work and deliverables.

For a share project, edits already affect host files. Only the host writes
its protected top-level Git metadata. Do not commit or change that metadata
from the box. Nested independent repositories have writable Git metadata;
that does not protect their files from concurrent edits.

For a disk project, commit work in the box. The user runs `pinfold fetch`
to bring those branches to the host. Uncommitted and home-only files do not
cross that way; the user can explicitly export retained disk files.

These instructions are guidance. Pinfold enforces the filesystem boundary.
