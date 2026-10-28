#!/usr/bin/python3
import io,os,tarfile
from pathlib import Path
p=Path("/tmp/fixture");p.mkdir(exist_ok=True)
(p/"busybox").write_bytes(Path("/bin/busybox").read_bytes());os.chmod(p/"busybox",0o755)
with tarfile.open(p/"fixture.tar","w",format=tarfile.PAX_FORMAT) as t:
    for name,data,uid,gid,mode,xattrs in [
        ("owned",b"owner proof\n",123,456,0o751,{"SCHILY.xattr.user.phase6":"yes"}),
        ("sbin/probe-init",b"#!/bin/busybox sh\n/bin/busybox mount -t proc proc /proc\n/bin/busybox echo CONVERTED_ROOT_BOOTED\n/bin/busybox stat -c '%u:%g %a %h %n' /owned /hard\n/bin/busybox readlink /sym\n/bin/busybox cat /dir/new\n/bin/busybox test ! -e /gone && /bin/busybox echo WHITEOUT_OK\n/bin/busybox cat /proc/mounts\n/bin/busybox poweroff -f\n",0,0,0o755,{})]:
        info=tarfile.TarInfo(name);info.uid=uid;info.gid=gid;info.mode=mode;info.size=len(data);info.pax_headers=xattrs
        t.addfile(info,io.BytesIO(data))
(p/"Dockerfile").write_text("""FROM scratch
COPY busybox /bin/busybox
ADD fixture.tar /
RUN ["/bin/busybox", "sh", "-c", "/bin/busybox ln /owned /hard; /bin/busybox ln -s /owned /sym; /bin/busybox echo old > /gone; /bin/busybox mkdir /dir; /bin/busybox echo old > /dir/old; /bin/busybox cat /proc/self/status"]
RUN ["/bin/busybox", "sh", "-c", "/bin/busybox rm /gone; /bin/busybox rm -rf /dir; /bin/busybox mkdir /dir; /bin/busybox echo newer > /dir/new"]
ENV PHASE6_VALUE=preserved
LABEL example.phase="6"
""")
