#!/usr/bin/python3
"""Scratch format and validator for the phase-6 evidence trial, not Pinfold."""
import hashlib,json,os,re,shutil,struct,sys,tempfile
from pathlib import Path
MAGIC=b"PINFOLD-OCI-TRIAL-1\n"
LIMIT=512*1024*1024
META=1024*1024
MAX_BLOBS=4096
def digest(data):return "sha256:"+hashlib.sha256(data).hexdigest()
def emit(layout,out):
    with open(out,"wb") as f:
        f.write(MAGIC)
        paths=[Path(layout)/"oci-layout",Path(layout)/"index.json",*sorted((Path(layout)/"blobs/sha256").iterdir())]
        for p in paths:
            data=p.read_bytes()
            kind=p.name if p.name in ("oci-layout","index.json") else "blob"
            header=json.dumps({"kind":kind,"digest":digest(data),"size":len(data)},separators=(",",":")).encode()
            f.write(struct.pack("!I",len(header)));f.write(header);f.write(data)
        f.write(struct.pack("!I",0))
def exact(f,n):
    b=f.read(n)
    if len(b)!=n:raise ValueError("truncated")
    return b
def strict_json(b):
    def pairs(kvs):
        d={}
        for k,v in kvs:
            if k in d:raise ValueError("duplicate JSON key")
            d[k]=v
        return d
    return json.loads(b,object_pairs_hook=pairs)
def ingest(source,store):
    store=Path(store);store.mkdir(mode=0o700,parents=True,exist_ok=True)
    staging=Path(tempfile.mkdtemp(prefix=".incoming-",dir=store))
    try:
        objects={};total=0
        with open(source,"rb") as f:
            if exact(f,len(MAGIC))!=MAGIC:raise ValueError("magic")
            for count in range(MAX_BLOBS+3):
                n=struct.unpack("!I",exact(f,4))[0]
                if n==0:break
                if n>8192:raise ValueError("header bound")
                h=strict_json(exact(f,n))
                if set(h)!={"kind","digest","size"}:raise ValueError("header fields")
                kind=h["kind"];dg=h["digest"];size=h["size"]
                if kind not in ("oci-layout","index.json","blob"):raise ValueError("kind")
                if type(dg)!=str or not re.fullmatch("sha256:[0-9a-f]{64}",dg):raise ValueError("digest syntax")
                if type(size)!=int or size<0 or size>LIMIT:raise ValueError("size bound")
                if kind!="blob" and size>META:raise ValueError("metadata bound")
                total+=size
                if total>LIMIT:raise ValueError("total bound")
                name=kind if kind!="blob" else dg
                if name in objects:raise ValueError("duplicate frame")
                p=staging/kind if kind!="blob" else staging/"blobs"/"sha256"/dg[7:]
                p.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
                sha=hashlib.sha256()
                with p.open("xb") as target:
                    os.chmod(p,0o600)
                    left=size
                    while left:
                        b=exact(f,min(left,1024*1024));target.write(b);sha.update(b);left-=len(b)
                    target.flush();os.fsync(target.fileno())
                if "sha256:"+sha.hexdigest()!=dg:raise ValueError("content digest")
                objects[name]=(p,size)
            else:raise ValueError("frame count bound")
            if f.read(1):raise ValueError("trailing bytes")
        def metadata(name):
            p,n=objects[name]
            if n>META:raise ValueError("JSON bound")
            return strict_json(p.read_bytes())
        if metadata("oci-layout")!={"imageLayoutVersion":"1.0.0"}:raise ValueError("layout")
        seen=set()
        def reference(d):
            if not isinstance(d,dict) or "urls" in d:raise ValueError("external descriptor")
            dg=d["digest"];size=d["size"]
            if dg not in objects or type(size)!=int or objects[dg][1]!=size:raise ValueError("reference")
            seen.add(dg);return dg
        index=metadata("index.json")
        if index.get("schemaVersion")!=2 or len(index.get("manifests",[]))!=1:raise ValueError("single image required")
        desc=index["manifests"][0]
        if desc.get("mediaType")!="application/vnd.oci.image.manifest.v1+json":raise ValueError("manifest media")
        mid=reference(desc);manifest=metadata(mid)
        if manifest.get("schemaVersion")!=2:raise ValueError("manifest schema")
        if manifest["config"].get("mediaType")!="application/vnd.oci.image.config.v1+json":raise ValueError("config media")
        cfg=metadata(reference(manifest["config"]))
        if (cfg.get("os"),cfg.get("architecture"))!=("linux","amd64"):raise ValueError("platform")
        layers=manifest["layers"]
        if len(layers)>MAX_BLOBS:raise ValueError("layer count")
        for layer in layers:
            if layer["mediaType"] not in ("application/vnd.oci.image.layer.v1.tar+gzip","application/vnd.oci.image.layer.v1.tar"):raise ValueError("layer media")
            reference(layer)
        diffs=cfg.get("rootfs",{}).get("diff_ids",[])
        if len(diffs)!=len(layers) or any(not re.fullmatch("sha256:[0-9a-f]{64}",x) for x in diffs):raise ValueError("diff_ids")
        if set(objects)-{"oci-layout","index.json"}!=seen:raise ValueError("unreferenced object")
        # This model publishes one validated immutable generation. Names/catalog are unimplemented.
        published=store/mid[7:]
        if published.exists():shutil.rmtree(staging)
        else:staging.rename(published)
        return {"manifest":mid,"config":manifest["config"]["digest"],"layers":len(layers),"bytes":total,"labels":cfg.get("config",{}).get("Labels",{}),"env":cfg.get("config",{}).get("Env",[])}
    except BaseException:
        shutil.rmtree(staging,ignore_errors=True)
        raise
if __name__=="__main__":
    if sys.argv[1]=="emit":emit(sys.argv[2],sys.argv[3])
    else:print(json.dumps(ingest(sys.argv[2],sys.argv[3])))
