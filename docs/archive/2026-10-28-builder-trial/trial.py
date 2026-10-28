#!/usr/bin/python3
"""Disposable trial runner. Does not implement Pinfold's jail or image service."""
import hashlib,json,os,select,socket,subprocess,sys,time
from pathlib import Path
import proxy
os.chdir(Path(__file__).resolve().parent)
plan=json.loads(Path(sys.argv[1]).read_text())
for path in ["vsock","vsock_3128"]:
    try:Path(path).unlink()
    except FileNotFoundError:pass
if os.environ.get("TRIAL_DENY")=="1":proxy.ALLOW.clear()
proxy_server=proxy.start(str(Path("vsock_3128").resolve()))
config={"boot-source":{"kernel_image_path":str(Path("vmlinux").resolve()),"boot_args":"console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda ro init=/sbin/trial-init"+(" TRIAL_SNAPSHOTTER="+sys.argv[2] if len(sys.argv)>2 else "")},"drives":[{"drive_id":"root","path_on_host":str(Path("root.ext4").resolve()),"is_root_device":True,"is_read_only":True},{"drive_id":"cache","path_on_host":str(Path(os.environ.get("TRIAL_CACHE","cache.ext4")).resolve()),"is_root_device":False,"is_read_only":False}],"machine-config":{"vcpu_count":2,"mem_size_mib":2560},"vsock":{"guest_cid":3,"uds_path":str(Path("vsock").resolve())}}
if os.environ.get("TRIAL_INPUT"):
    config["drives"].append({"drive_id":"input","path_on_host":str(Path(os.environ["TRIAL_INPUT"]).resolve()),"is_root_device":False,"is_read_only":True})
Path("vm.json").write_text(json.dumps(config))
t=time.monotonic()
vm=subprocess.Popen(["./release-v1.17.0-x86_64/firecracker-v1.17.0-x86_64","--no-api","--config-file","vm.json"],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
log=open("console-"+Path(sys.argv[1]).stem+".log","wb",buffering=0)
try:
    deadline=time.monotonic()+60
    line=b""
    while time.monotonic()<deadline:
        if not select.select([vm.stdout],[],[],max(0,deadline-time.monotonic()))[0]:raise TimeoutError("boot")
        b=os.read(vm.stdout.fileno(),4096)
        if not b:raise RuntimeError("VM exited during boot")
        log.write(b); line+=b
        if b"TRIAL_READY" in line:break
    else:raise TimeoutError("boot")
    print(json.dumps({"boot_seconds":round(time.monotonic()-t,3)}),flush=True)
    for i,req in enumerate(plan):
        c=socket.socket(socket.AF_UNIX);c.settimeout(req.get("timeout",300)+30)
        c.connect(str(Path("vsock").resolve()));c.sendall(b"CONNECT 5000\n")
        ack=b""
        while not ack.endswith(b"\n"):ack+=c.recv(1)
        if not ack.startswith(b"OK "):raise RuntimeError(ack)
        c.sendall(json.dumps(req).encode()+b"\n")
        f=c.makefile("rb")
        header=f.readline(131072)
        if len(header)>=131072:raise ValueError("response header cap")
        result=json.loads(header);result["name"]=req["name"]
        if "size" in result:
            n=result["size"]
            if type(n) is not int or not 0<=n<=512*1024*1024:raise ValueError("size cap")
            out=Path("outputs")/(req["name"]+".bin")
            out.parent.mkdir(exist_ok=True)
            partial=out.with_suffix(".partial");h=hashlib.sha256()
            with partial.open("wb") as target:
                left=n
                while left:
                    data=f.read(min(left,1024*1024))
                    if not data:raise EOFError("truncated artifact")
                    target.write(data);h.update(data);left-=len(data)
                target.flush();os.fsync(target.fileno())
            if h.hexdigest()!=result["sha256"]:raise ValueError("digest mismatch")
            partial.replace(out);result["received"]=str(out)
        c.close()
        print(json.dumps(result),flush=True)
finally:
    vm.terminate()
    try:vm.wait(timeout=10)
    except subprocess.TimeoutExpired:vm.kill();vm.wait()
    rest=vm.stdout.read();log.write(rest);log.close()
    proxy_server.shutdown()
