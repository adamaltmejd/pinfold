#!/usr/bin/python3
import json,os,select,subprocess,time
cmd=["setpriv","--no-new-privs","--bounding-set=-sys_admin","buildkitd","--addr","unix:///run/no-admin.sock","--root","/var/lib/buildkit/no-admin","--oci-worker-snapshotter=native","--oci-worker-net=host","--containerd-worker=false","--oci-worker-gc=false"]
p=subprocess.Popen(cmd,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
log=b""
try:
    deadline=time.monotonic()+20
    while time.monotonic()<deadline:
        ready,_,_=select.select([p.stdout],[],[],max(0,deadline-time.monotonic()))
        if not ready:raise TimeoutError("daemon readiness")
        b=os.read(p.stdout.fileno(),4096)
        if not b:raise RuntimeError(log.decode())
        log+=b
        if b"running server on" in log:break
    else:raise TimeoutError("daemon readiness")
    result=subprocess.run(["buildctl","--addr","unix:///run/no-admin.sock","build","--frontend","dockerfile.v0","--local","context=/tmp/fixture","--local","dockerfile=/tmp/fixture","--output","type=oci,dest=/tmp/no-admin.tar"],capture_output=True,text=True,timeout=60)
    print(json.dumps({"build_returncode":result.returncode,"build_log":result.stdout+result.stderr,"daemon_log":log.decode()}))
finally:
    p.terminate();p.wait(timeout=10)
