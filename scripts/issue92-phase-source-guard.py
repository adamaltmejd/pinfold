#!/usr/bin/env python3
"""Read-only guard: fixed production source and exact reviewed test timing blobs."""
import hashlib,json,os,subprocess
from pathlib import Path
BASE='30725e08e56e96b205b55e61bab3097a4bd1c93d'
EXPECTED={'crates/e2e/src/lib.rs': '2bb7c24d929542027952c6fbbad666a9bd62ad95e7080e5652f888e23c22bf89', 'crates/e2e/src/timing.rs': 'c9b67b1d4b10a6100fea951a3a6e2e3e3f348b9a6294494ecaf68be59cc2ea61', 'crates/e2e/tests/e2e/box_.rs': '0e8724dcc7e17ad566fc1866a38996448cd4240670242dbf7ffa7d9e92497a46', 'crates/e2e/tests/e2e/cli.rs': 'f89990fcd7dc95b355fdf45f98e3aee739d174497089d7d811fdfa98788e6a63', 'crates/e2e/tests/e2e/image_warning.rs': '3124cd98056ee8fa520144c19ae61b3a5e277f218571043ed04b4845ee806780', 'crates/e2e/tests/e2e/main.rs': 'f6b0569b7be1309f712c8fabdc219524ce7541e98d8e84d782b0d8306beb8705', 'crates/e2e/tests/e2e/pi.rs': 'd873d2b6f7b40d09013242d4dd8c6331726b717e71018a589bb68394ef7e9d96', 'crates/e2e/tests/e2e/update.rs': '75d3bd6cf1391bf4da1d87889e888608f373f4b65e294c72d4cfe0c249e6baca', 'scripts/diagnose-suite-host.py': '69c4254fa9abbddb8d110a7995a16d32098504f4091e0bad0783b9198d3dab3a'}
EXPECTED['scripts/issue92-native-step-probe.py']='389a012eebd95232712a19b3eeb67f421568cc0d117f3cd4232df0f4666f0af8'
ALLOWED=set(EXPECTED)|{'.github/workflows/ci.yml','scripts/issue92-phase-source-guard.py'}
changed=set(subprocess.check_output(['git','diff','--name-only','-z',BASE,'--']).decode().split('\0'))-{''}
untracked=set(subprocess.check_output(['git','ls-files','--others','--exclude-standard','-z']).decode().split('\0'))-{''}
assert (changed|untracked)<=ALLOWED, 'undeclared source changes: '+repr(sorted((changed|untracked)-ALLOWED))
for name,digest in EXPECTED.items():
    assert hashlib.sha256(Path(name).read_bytes()).hexdigest()==digest, 'unexpected instrumentation: '+name
present={name:bool(os.environ.get(name)) for name in ('RUST_TEST_THREADS','CARGO_BUILD_JOBS','CARGO_TARGET_DIR')}
assert not any(present.values()), 'worker/target overrides present'
assert len(os.sched_getaffinity(0))==4, 'expected ordinary four-CPU affinity'
print(json.dumps({'baseline_revision':BASE,'source_revision':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
 'instrumentation_sha256':EXPECTED,'changed_paths':sorted(changed|untracked),'worker_overrides_present':present,
 'affinity_cpus':sorted(os.sched_getaffinity(0)), 'scope':'diagnosis only; no acceptance claim'},indent=2))
