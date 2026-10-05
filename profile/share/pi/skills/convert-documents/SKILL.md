---
name: convert-documents
description: Convert local PDF and Office documents to Markdown for reading with AnyDoc.
---

Use the installed `anydoc` command through bash. The input must be inside
the project or another mounted directory. Check the path if conversion
reports that the file is missing.

Run `anydoc "document.pdf" -o /tmp/document.md`, then read the Markdown.
With `-o`, successful conversion writes the file and may print nothing.
Use a fresh directory from `mktemp -d` when handling several documents.
Write output into the project when it should survive the box exiting.

Conversion may lose table structure, equations, figures, or reading order.
For PDFs, inspect important pages with the Poppler tools described in the
`read-pdf` skill. Scanned documents may lack extractable text; report the
limitation rather than inferring the content.

All conversion is local. Do not use hosted OCR or upload documents to a
conversion service.
