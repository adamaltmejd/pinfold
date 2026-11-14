#!/usr/bin/env python3
"""Measure sparse converter backing allocation without reading guest disks."""
import os, pathlib, subprocess, sys, threading, time

root = pathlib.Path(__file__).resolve().parent
cache = pathlib.Path(os.environ.get('XDG_CACHE_HOME', pathlib.Path.home()/'.cache'))/'pinfold/builder'
peaks = {}
finished = threading.Event()

def sample():
    while not finished.is_set():
        for disk in cache.glob('*/job/converter.ext4'):
            try: peaks[str(disk)] = max(peaks.get(str(disk), 0), disk.stat().st_blocks*512)
            except FileNotFoundError: pass
        finished.wait(0.1)

thread = threading.Thread(target=sample); thread.start()
try:
    args = ['project'] if sys.argv[1:] == ['project'] else ['build', *sys.argv[1:]]
    result = subprocess.run([sys.executable, str(root/'probe.py'), *args])
finally:
    finished.set(); thread.join()
print('observed-converter-allocated-peak-bytes='+str(max(peaks.values(), default=0)), flush=True)
sys.exit(result.returncode)
