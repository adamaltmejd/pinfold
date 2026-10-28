#!/usr/bin/python3
"""Malicious-frame experiments against the scratch validator only."""
import hashlib,json,struct,tempfile
from pathlib import Path
import frames
def record(kind,data,extra=None,size=None):
    h={"kind":kind,"digest":frames.digest(data),"size":len(data) if size is None else size}
    if extra:h.update(extra)
    b=json.dumps(h).encode();return struct.pack("!I",len(b))+b+data
source=Path("outputs/fixture.bin").read_bytes()
nh=struct.unpack("!I",source[len(frames.MAGIC):len(frames.MAGIC)+4])[0]
headstart=len(frames.MAGIC)+4
h=json.loads(source[headstart:headstart+nh])
firstend=headstart+nh+h["size"]
bad_digest=bytearray(source);bad_digest[headstart+nh]^=1
cases={
    "truncated":source[:-64],
    "wrong-digest":bytes(bad_digest),
    "duplicate-frame":source[:firstend]+source[len(frames.MAGIC):firstend]+source[firstend:],
    "over-size":frames.MAGIC+record("blob",b"",size=frames.LIMIT+1),
    "path-kind":frames.MAGIC+record("../../escape",b"hello")+struct.pack("!I",0),
    "ownership-field":frames.MAGIC+record("blob",b"hello",{"uid":0})+struct.pack("!I",0),
    "trailing":source+b"x",
    "missing-blobs":source[:firstend]+struct.pack("!I",0),
}
store=Path("hostile-catalog")
good=frames.ingest("outputs/fixture.bin",store)
def snapshot():
    return {str(p.relative_to(store)):hashlib.sha256(p.read_bytes()).hexdigest() for p in store.rglob("*") if p.is_file()}
before=snapshot()
results=[]
with tempfile.TemporaryDirectory() as d:
    for name,data in cases.items():
        p=Path(d)/name;p.write_bytes(data)
        try:frames.ingest(p,store)
        except (ValueError,KeyError,TypeError,json.JSONDecodeError) as e:results.append({"case":name,"refused":str(e)})
        else:raise AssertionError(name+" accepted")
        assert snapshot()==before,name+" changed catalog"
print(json.dumps({"valid":good,"cases":results,"prior_generation_unchanged":snapshot()==before},indent=2))
