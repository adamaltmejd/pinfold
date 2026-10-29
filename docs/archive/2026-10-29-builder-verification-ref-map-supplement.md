# Supplement: retained corruption-case reference maps

Date: 2026-10-29. Later operator trial of the unchanged Ring executable
from `617de586b9d7a588e4fb611a438511ce4fe73b25`. This separately closes
the raw-map evidence gap recorded in the earlier validation note; it does
not change that original run's retained evidence.

A same-size one-byte corruption of the installed bootstrap root produced
exit 1 and `builder-integrity: checksum mismatch` in **2.649201622 s**.
The retained `refs-before.json` and `refs-after-refusal.json` each contain
seven entries and are byte-identical, SHA-256
`8235fa09e2649fed3b43ec040f47162aa0ee9172b944c3b9173acbb86154b830`.
Their equality was independently checked from the exported raw files.

The full root hash before corruption and after restoration is the pin
`ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
The restored ordinary positive build passed in **9.992592674 s**; its
stdio and resulting reference map are also retained. Executable hashes
before/after match
`f9ff3c007b971722b2813d6749cd4a2d634ae756575cd9c22c9f457077e8c9ca`.

This is a narrow integrity/ref-atomicity trial, not another full suite or
a new fresh-download capture. It establishes no universal latency or
CI-budget result. No production change, test or VM operation was performed
while preparing this supplemental note.

Evidence root:
`/Users/adam/.codex/worktrees/ed8e/pinfold/.local-evidence/critical-five-latency-2026-10-29/`.

| File | SHA-256 |
|---|---|
| `ring-ref-map-proof.tar.gz` | `767dd295824deb2c731088e732641c168d91a386dd1fd967f4f9daef539c614d` |
| `corruption-maps.py` | `a4b3f552aeb377f718302af9895d9b88a05d3b3517e2f5893443acb137dd3c6c` |
