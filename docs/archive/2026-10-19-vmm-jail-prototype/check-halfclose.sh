#!/bin/bash
# Does a host half-close of a host-initiated vsock connection reach the
# guest as EOF, or end the connection? Control: same request, no half-close.
. ~/proto/lib.sh
mkdata $ST/h.ext4 64M
up h A $ST/h.ext4 >/dev/null
cat >$ST/hc.py <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall(b"CONNECT 1024\nsleep 1; echo after-sleep\n")
if sys.argv[2] == "halfclose": s.shutdown(socket.SHUT_WR)
t = time.time(); out = b""
while (d := s.recv(4096)): out += d
print(sys.argv[2], f"{(time.time()-t)*1000:.0f} ms", out)
PY
python3 $ST/hc.py $ST/h/vsock/v.sock keep-open
python3 $ST/hc.py $ST/h/vsock/v.sock halfclose
kill_vm h
