---
name: read-pdf
description: Read local PDFs, extracting text or inspecting rendered pages for scans, figures, and tables.
---

Use the installed AnyDoc and Poppler tools through bash. The PDF must be
inside the project or another mounted directory. Check the path if a
command reports that the file is missing.

Inspect metadata and page count with `pdfinfo "paper.pdf"`. Prefer AnyDoc
Markdown extraction as described in the `convert-documents` skill.
For fallback text extraction or a selected page range, use
`pdftotext -layout -f FIRST -l LAST "paper.pdf" /tmp/paper.txt`, then read
the output. Use a fresh directory from `mktemp -d` for temporary outputs
when handling several documents.

Scans may have no text layer. Figures and tables may lose meaning during
extraction. Render the relevant pages with
`pdftoppm -f FIRST -l LAST -scale-to 1600 -png "paper.pdf" /tmp/page`, then
use the read tool to inspect the resulting PNGs. Avoid rendering a whole
long document. Report unreadable content rather than inferring it.

All conversion is local. Do not upload documents to a conversion or OCR
service.
