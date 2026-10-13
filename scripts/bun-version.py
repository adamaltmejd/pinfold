#!/usr/bin/env python3
"""Read the candidate's image pin for CI's Bun runtime."""

import re
from pathlib import Path

recipe = (
    Path(__file__).resolve().parent.parent / "profile/image/bun.Containerfile"
).read_text()
versions = set(re.findall(r"/bun-v(\d+\.\d+\.\d+)/", recipe))
if len(versions) != 1:
    raise SystemExit("Bun image pins must name one version")
print(f"bun={versions.pop()}")
