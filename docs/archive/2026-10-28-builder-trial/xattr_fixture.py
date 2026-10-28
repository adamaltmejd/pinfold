#!/usr/bin/python3
"""Golden OCI metadata fixture outside BuildKit's ADD behavior."""
import gzip,hashlib,io,json,shutil,tarfile
from pathlib import Path
base=Path("catalog-model/74d897e251664ee57e05319581aa572414231fe1e40f02f809d6330c26a9e632")
target=Path("xattr-layout")
if target.exists():shutil.rmtree(target)
shutil.copytree(base,target)
def dg(b):return "sha256:"+hashlib.sha256(b).hexdigest()
def write(data):
    d=dg(data);(target/"blobs/sha256"/d[7:]).write_bytes(data);return d,len(data)
index=json.loads((target/"index.json").read_text())
m=json.loads((target/"blobs/sha256"/index["manifests"][0]["digest"][7:]).read_text())
cfg=json.loads((target/"blobs/sha256"/m["config"]["digest"][7:]).read_text())
b=io.BytesIO()
with tarfile.open(fileobj=b,mode="w",format=tarfile.PAX_FORMAT) as t:
    content=b"owner proof\n"
    f=tarfile.TarInfo("owned");f.uid=123;f.gid=456;f.mode=0o751;f.size=len(content);f.pax_headers={"SCHILY.xattr.user.phase6":"yes"}
    t.addfile(f,io.BytesIO(content))
    f=tarfile.TarInfo("hard");f.uid=123;f.gid=456;f.mode=0o751;f.type=tarfile.LNKTYPE;f.linkname="owned";f.pax_headers={"SCHILY.xattr.user.phase6":"yes"};t.addfile(f)
raw=b.getvalue();compressed=gzip.compress(raw,mtime=0)
ld,ls=write(compressed)
m["layers"].append({"mediaType":"application/vnd.oci.image.layer.v1.tar+gzip","digest":ld,"size":ls})
cfg["rootfs"]["diff_ids"].append(dg(raw));cfg.setdefault("history",[]).append({"created_by":"phase6 golden xattr fixture"})
old={index["manifests"][0]["digest"],m["config"]["digest"]}
cd,cs=write(json.dumps(cfg,separators=(",",":")).encode());m["config"].update(digest=cd,size=cs)
md,ms=write(json.dumps(m,separators=(",",":")).encode());index["manifests"][0].update(digest=md,size=ms)
(target/"index.json").write_text(json.dumps(index,separators=(",",":")))
for d in old:(target/"blobs/sha256"/d[7:]).unlink()
print(json.dumps({"manifest":md,"layer":ld,"diff_id":dg(raw),"xattr":"user.phase6=yes"}))
