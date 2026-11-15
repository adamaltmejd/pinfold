#!/usr/bin/env python3
"""Real native startup failure; diagnostics stay in memory, never in this log."""
import json
import os
from pathlib import Path
import subprocess as sp
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
image = sys.argv[2]
w = Path(tempfile.mkdtemp(prefix='pfrt-startup.', dir='/tmp')).resolve()
(w/'s/pinfold').mkdir(parents=True)
(w/'s/pinfold/maintenance').touch()
env = {k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update(XDG_CONFIG_HOME=str(w/'c'), XDG_STATE_HOME=str(w/'s'), PINFOLD_NO_UPDATE_CHECK='1')
plan = dict(name='probe-startup-'+str(os.getpid()), image=image, cpus=2, memory='1G')
# The real native label parser refuses the second '=' and echoes its input.
value = 'first=second\npinfold: FORGED\x1b[31m' + 'X'*98304 + 'OMITTED-END'
plan['labels'] = {'probe-error':value}
r = sp.run([binary,'box','up'], input=json.dumps(plan).encode()+b'\n', env=env, capture_output=True, timeout=90)
refusal = json.loads(r.stdout)
detail = refusal['detail']
assert r.returncode == 1 and refusal['event'] == 'failed'
assert 'invalid label' in detail.lower(), 'native diagnostic absent'
assert 'FORGED' in detail and '\\n' in detail and '\\u{1b}' in detail
assert '\n' not in detail and '\x1b' not in detail and 'OMITTED-END' not in detail
assert detail.count('X') <= 65536 and detail.count('X') > 60000
for path in (w/'s').rglob('*'):
    if path.is_file():
        data = path.read_bytes()
        assert b'FORGED' not in data and b'invalid label' not in data.lower()
print('native run startup failure: exit=1; native diagnostic surfaced; newline/ESC escaped; 96KiB input truncated at 64KiB; state contains no diagnostic', flush=True)
plan['labels'] = {'probe-error':'control'}
child = sp.Popen([binary,'box','up'], env=env, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
child.stdin.write(json.dumps(plan).encode()+b'\n'); child.stdin.flush()
ready = json.loads(child.stdout.readline())
assert ready['event'] == 'ready'
# A guest that stops reading its input must still return its real status.
result = sp.run([binary,'box','exec',plan['name'],'--','sh','-c','exit 23'], input=b'x'*1048576, env=env, capture_output=True, timeout=60)
assert result.returncode == 23
print('non-TTY early stdin closure: 1MiB input, observed exit=23', flush=True)
result = sp.run([binary,'box','exec',plan['name'],'--','sh','-c','exec 1>&-; wc -c >/tmp/count; cat /tmp/count >&2'], input=b'x'*1048576, env=env, capture_output=True, timeout=60)
assert result.returncode == 0 and not result.stdout and result.stderr.strip() == b'1048576'
print('non-TTY independent input: guest closed stdout, read all 1MiB, separate stderr count=1048576, exit=0', flush=True)
child.stdin.close(); child.stdin=None
out, err = child.communicate(timeout=90)
assert child.returncode == 0 and json.loads(out)['reason']=='stdin-closed'
print('corrected native startup: ready; stdin EOF teardown exit=0', flush=True)
