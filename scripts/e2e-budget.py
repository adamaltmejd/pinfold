#!/usr/bin/env python3
"""Run the full e2e suite, then enforce the CI time budget."""
import json
import subprocess
import sys
import time

started = time.monotonic_ns()
try:
    exit_code = subprocess.run(['cargo', 'test', '-p', 'e2e', '--locked'], check=False).returncode
except OSError as error:
    print(error, file=sys.stderr)
    exit_code = 127
elapsed_ns = time.monotonic_ns() - started
print(json.dumps({'e2e_elapsed_ns': elapsed_ns, 'e2e_exit': exit_code}), flush=True)
raise SystemExit(exit_code or (elapsed_ns >= 300_000_000_000))
