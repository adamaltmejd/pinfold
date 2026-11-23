---
name: read-documents
description: Read local PDF, Office and other documents. Convert them to Markdown with AnyDoc, or extract and render PDF pages with Poppler.
---

Everything here runs locally through bash. Never upload a document to a
hosted conversion or OCR service. The document must be inside the project
or another mounted directory.

Convert to Markdown, then read the result:

    out=$(mktemp -d)
    anydoc "report.pdf" -o "$out/report.md"

Success may print nothing. Write into the project instead when the output
should outlive the box.

Conversion can lose tables, equations, figures and reading order. Check
the pages that matter in a PDF with Poppler:

- `pdfinfo "report.pdf"`: metadata and page count.
- `pdftotext -layout -f 3 -l 4 "report.pdf" "$out/pages.txt"`: text of
  pages 3 to 4.
- `pdftoppm -f 3 -l 4 -scale-to 1600 -png "report.pdf" "$out/page"`:
  render pages 3 to 4, then look at the PNGs with the read tool. Render
  only the pages you need.

A scanned page may have no text layer; AnyDoc then exits 3 and names it.
Look at its rendering, and report what stays unreadable rather than
guessing.
