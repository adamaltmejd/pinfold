#!/usr/bin/env python3
"""Sanitize recorded text and compress raw syscall traces without loss."""
import gzip
from pathlib import Path
import re

OUT = Path(__file__).resolve().parent
for path in OUT.rglob('*'):
    if path.is_file() and path.suffix in ['.txt', '.json', '.jsonl', '.py', '.html', '.Containerfile']:
        original = path.read_text()
        value = original.replace(str(Path.home()), '/Users/user')
        value = re.sub(r'AIza[0-9A-Za-z_-]{35}', '[REDACTED_BUNDLED_VENDOR_KEY]', value)
        if value != original:
            path.write_text(value)
for path in OUT.rglob('*-strace.txt'):
    raw = path.read_bytes()
    packed = gzip.compress(raw, mtime=0)
    assert gzip.decompress(packed) == raw
    path.with_suffix('.txt.gz').write_bytes(packed)
    path.unlink()
