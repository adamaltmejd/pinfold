#!/usr/bin/env python3
"""Reuse long-root lifecycle with the approved dead-claim recovery contract."""
from pathlib import Path
base=Path(__file__).resolve().parent.parent
source=(base/'2026-11-16-runtime-transport/lifecycle.py').read_text()
a=source.index('    # Only our orphan directory is changed')
b=source.index('    # Reuse the original PR 1 CLI cases')
replacement='''    # An unverifiable dead prefix remains byte for byte; its claim retires.
    marker=runtime/'generation'
    original=marker.read_bytes()
    marker.write_bytes(original[:17])
    def snapshot():
        return {str(p.relative_to(runtime)): (p.lstat().st_mode, p.read_bytes() if p.is_file() else None)
                for p in runtime.rglob('*')}
    before=snapshot()
    run(['box','prune'])
    assert snapshot()==before
    _,replacement,_=start(spec(name,disk=True))
    assert replacement['run_id']!=ready['run_id'] and snapshot()==before
    assert exec_(name,'cat','/data/marker')==b'durable'
    finish(name)
    marker.write_bytes(original)
    run(['box','prune'])
    assert not runtime.exists() and live_dir.stat().st_ino==live_identity
    assert exec_(witness,'printf','LIVE-SURVIVES')==b'LIVE-SURVIVES'
    emit('A/B long-root crash recovery: unverifiable old prefix preserved byte for byte; fresh generation ran; restored complete marker allowed later cleanup; live witness survived')
    finish(witness,eof=True)
'''
source=source[:a]+replacement+source[b:]
# Preserve the original driver's relative references when evaluating it here.
exec(compile(source,str(base/'2026-11-16-runtime-transport/lifecycle.py'),'exec'),dict(globals(),__file__=str(base/'2026-11-16-runtime-transport/lifecycle.py')))
