#!/usr/bin/python3
"""Disposable phase-6 RPC harness. Root command execution; never production code."""
import hashlib,json,os,socket,subprocess,time,threading
from pathlib import Path

# Consume the daemon's readiness signal before advertising the trial RPC.
daemon_log=open("/run/buildkit-log","rb",buffering=0)
daemon_file=open("/tmp/buildkitd.log","wb",buffering=0)
while True:
    line=daemon_log.readline()
    if not line:raise RuntimeError("BuildKit ended before readiness")
    daemon_file.write(line)
    if b"running server on" in line:break
def drain():
    while data:=daemon_log.read(65536):daemon_file.write(data)
threading.Thread(target=drain,daemon=True).start()
s=socket.socket(socket.AF_VSOCK,socket.SOCK_STREAM)
s.bind((socket.VMADDR_CID_ANY,5000)); s.listen(1)
print("TRIAL_READY",flush=True)
while True:
    c,_=s.accept()
    try:
        f=c.makefile("rwb",buffering=0)
        line=f.readline(1024*1024)
        if len(line)>=1024*1024: raise ValueError("request too large")
        req=json.loads(line)
        for name,data in req.get("files",{}).items():
            import base64
            p=Path(name); p.parent.mkdir(parents=True,exist_ok=True)
            p.write_bytes(base64.b64decode(data))
        t=time.monotonic()
        with open("/tmp/command.log","wb") as log:
            try:
                p=subprocess.run(["/bin/bash","-c",req["command"]],stdout=log,stderr=subprocess.STDOUT,timeout=req.get("timeout",300))
                rc=p.returncode
            except subprocess.TimeoutExpired:
                rc=124
        with open("/tmp/command.log","rb") as log:
            log.seek(0,2); n=log.tell(); log.seek(max(0,n-65536)); tail=log.read().decode(errors="replace")
        res={"returncode":rc,"seconds":round(time.monotonic()-t,3),"log":tail}
        artifact=Path(req["artifact"]) if rc==0 and req.get("artifact") else None
        if artifact:
            n=artifact.stat().st_size
            if n>512*1024*1024: raise ValueError("trial output exceeds 512 MiB")
            h=hashlib.file_digest(artifact.open("rb"),"sha256").hexdigest()
            res.update(size=n,sha256=h)
        f.write(json.dumps(res).encode()+b"\n")
        if artifact:
            with artifact.open("rb") as af:
                while b:=af.read(1024*1024): f.write(b)
    except Exception as e:
        try:c.sendall(json.dumps({"error":str(e)}).encode()+b"\n")
        except OSError:pass
    finally:c.close()
