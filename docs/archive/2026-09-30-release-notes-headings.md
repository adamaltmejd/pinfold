# Release note headings

Publication verification found that Git's default annotated-tag cleanup
removed Markdown heading lines from the v0.1.0 notes. The GitHub release
body was corrected from the prepared notes, including Caller changes.
The published tag was not rewritten.

Nightly tag creation now uses `--cleanup=verbatim`. Manual annotated tags
must also use this option when notes contain Markdown headings. A scratch
Git repository verifies that the annotation retains Caller changes.
The workflow passes actionlint and diff checks. This changes release
metadata only; the v0.1.0 binaries and tested production tree remain intact.
