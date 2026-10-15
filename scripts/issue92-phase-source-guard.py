#!/usr/bin/env python3
"""Verify exact temporary observations and unchanged release source."""
import hashlib, json, os, subprocess
from pathlib import Path
BASE = 'c19906a9f1aedd8c65fb7e4fab5bf3ebbc0ec5d2'
EXPECTED = {'crates/e2e/src/lib.rs': '30b08681cd3fc55282396e92e24fde6687ab3b985f7f4a3f4d55716ee2654915', 'crates/e2e/src/timing.rs': '2adeb5d395012e9fdfdf21db4bf9192ae011cc90a7c8241e16f2d72e84155526', 'crates/e2e/tests/e2e/box_.rs': '0e8724dcc7e17ad566fc1866a38996448cd4240670242dbf7ffa7d9e92497a46', 'crates/e2e/tests/e2e/cli.rs': 'f89990fcd7dc95b355fdf45f98e3aee739d174497089d7d811fdfa98788e6a63', 'crates/e2e/tests/e2e/image_warning.rs': '3124cd98056ee8fa520144c19ae61b3a5e277f218571043ed04b4845ee806780', 'crates/e2e/tests/e2e/main.rs': '926f84ca76fb78e4ceba1c1ba2a341b63e8af764c30086606590dacab8a1fd39', 'crates/e2e/tests/e2e/pi.rs': 'd873d2b6f7b40d09013242d4dd8c6331726b717e71018a589bb68394ef7e9d96', 'crates/e2e/tests/e2e/update.rs': '75d3bd6cf1391bf4da1d87889e888608f373f4b65e294c72d4cfe0c249e6baca', 'scripts/diagnose-suite-host.py': 'fdaa3a01faf45530c4dd582799753a3913d686ab1e8ea1b75ded467e9a53d47f', '.github/workflows/ci.yml': '856d3eedd313797b5e071d0a403c3ee7c3e61bb691efe5feb9241d32ad0da201'}
ALLOWED = set(EXPECTED) | {"scripts/issue92-phase-source-guard.py"}
changed = set(subprocess.check_output(["git", "diff", "--name-only", "-z", BASE, "--"]).decode().split("\0")) - {""}
untracked = set(subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "-z"]).decode().split("\0")) - {""}
assert (changed | untracked) <= ALLOWED, "undeclared source changes: " + repr(sorted((changed | untracked) - ALLOWED))
for name, digest in EXPECTED.items():
    assert hashlib.sha256(Path(name).read_bytes()).hexdigest() == digest, "unexpected instrumentation: " + name
present = {name: bool(os.environ.get(name)) for name in ("RUST_TEST_THREADS", "CARGO_BUILD_JOBS", "CARGO_TARGET_DIR")}
assert not any(present.values()), "worker/target overrides present"
assert len(os.sched_getaffinity(0)) == 4, "expected ordinary four-CPU affinity"
print(json.dumps({"baseline_revision": BASE, "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(), "instrumentation_sha256": EXPECTED, "changed_paths": sorted(changed | untracked), "worker_overrides_present": present, "affinity_cpus": sorted(os.sched_getaffinity(0)), "capture": "workflow supplies RUST_TEST_NOCAPTURE only to the complete unchanged budget command", "scope": "diagnosis only; no acceptance claim"}, indent=2))
