import datetime,gzip,hashlib,json,os,tarfile
from pathlib import Path
base=Path("/Users/adam/Code/pinfold/target/phase6-builder-trial-2026-10-28")
evidence=Path("/Users/adam/.codex/worktrees/phase6-builder-trial/pinfold/docs/archive/2026-10-28-builder-trial")
pins=json.loads((evidence/"pins.json").read_text())
outputs=json.loads((evidence/"artifact-manifest.json").read_text())
expected={**pins,**{"outputs/"+k:v for k,v in outputs.items()}}
source=base/"artifacts.tar.gz.partial"
if not source.exists():source=base/"artifacts.tar.gz"
seen=set();raw=0
with gzip.open(source,"rb") as stream:
    with tarfile.open(fileobj=stream,mode="r|") as archive:
        for m in archive:
            if m.isdir():
                assert m.name=="outputs",m.name
                continue
            assert m.isfile() and m.name in expected and m.name not in seen,m.name
            wanted=expected[m.name];assert m.size==wanted["bytes"],m.name
            h=hashlib.sha256();n=0
            f=archive.extractfile(m)
            while b:=f.read(8*1024*1024):h.update(b);n+=len(b)
            assert n==wanted["bytes"] and h.hexdigest()==wanted["sha256"],m.name
            seen.add(m.name);raw+=n
    # Consume padding/footer too, validating gzip CRC and EOF.
    while b:=stream.read(8*1024*1024):assert not b.strip(b"\0"),"trailing data"
assert seen==set(expected),set(expected)-seen
sha=hashlib.file_digest(source.open("rb"),"sha256").hexdigest()
destination=base/"artifacts.tar.gz"
os.replace(source,destination)
metadata=base/"evidence.tar.gz"
result={"verified_at_utc":datetime.datetime.now(datetime.timezone.utc).isoformat(),"archive":str(destination),"archive_bytes":destination.stat().st_size,"archive_sha256":sha,"verified_regular_members":len(seen),"raw_member_bytes":raw,"member_manifest":"docs/archive/2026-10-28-builder-trial/artifact-manifest.json and pins.json","metadata_archive":str(metadata),"metadata_bytes":metadata.stat().st_size,"metadata_sha256":hashlib.file_digest(metadata.open("rb"),"sha256").hexdigest(),"cache_disks_included":False,"extracted_or_mounted_guest_disks":False}
out=base/"verification.json"
with out.open("w") as f:json.dump(result,f,indent=2);f.write("\n");f.flush();os.fsync(f.fileno())
print(json.dumps(result,indent=2))
