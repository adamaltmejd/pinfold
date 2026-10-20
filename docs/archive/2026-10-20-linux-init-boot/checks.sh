#!/bin/bash
# checks.sh [INIT]: the Linux clauses of rows 1, 5, 6, 11 and 20, and the
# exec protocol, against the init disk ~/fc/INIT.ext4 (default `init`).
# Each check prints `== name` and what it observed.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
init=~/fc/${1:-init}.ext4
st=~/st
export PATH=$PATH:/usr/sbin:/sbin
pkill -9 -x firecracker 2>/dev/null; rm -rf "$st"; mkdir -p "$st"
fc() { python3 "$here/fcinit.py" "$@"; }
upbox() {
  local name=$1; shift
  fc up "$st/$name" --firecracker ~/fc/firecracker --kernel ~/fc/vmlinux \
    --init "$init" --root ~/fc/root.ext4 "$@" >"$st/$name.up" 2>&1 &
  until grep -qE "ready|failed|Error|Traceback" "$st/$name.up" 2>/dev/null; do sleep 0.1; done
  echo "up $name: $(head -1 "$st/$name.up")"
}
x() { local b=$1; shift; fc exec "$st/$b" "$@"; echo "[exit $?]"; }
down() {
  local name=$1 pid
  pid=$(cat "$st/$name/vmm.pid")
  echo "down $name: $(fc down "$st/$name")"
  while kill -0 "$pid" 2>/dev/null; do sleep 0.1; done
}

# Fixture: a `from` tree with a 644 file, a 755 script, a symlink, a FIFO
# and a nested dir; a read-only corpus disk; an empty 256M work disk.
mkdir -p "$st/from/sub" "$st/corpus"
echo a >"$st/from/a"; printf '#!/bin/sh\necho run\n' >"$st/from/run.sh"
chmod 644 "$st/from/a"; chmod 755 "$st/from/run.sh"
ln -s a "$st/from/link"; mkfifo "$st/from/pipe"; echo b >"$st/from/sub/b"
echo corpus >"$st/corpus/c"
mkfs.ext4 -q -d "$st/corpus" "$st/corpus.ext4" 16M
truncate -s 256M "$st/work.ext4"; mkfs.ext4 -q -F "$st/work.ext4"
export PINFOLD_ENV_SECRET="marker-$(od -An -N8 -tx8 /dev/urandom | tr -d ' ')"
upbox one --disk "$st/work.ext4:/work:rw" --disk "$st/corpus.ext4:/corpus:ro" \
  --fill "/work=$st/from" --env PINFOLD_ENV_SECRET --mem 512

echo "== row 1: interfaces"; x one -- ls /sys/class/net
echo "== row 1: vsock to CID 2 from inside (1024 exec, 1025 control, 4444 unused, 3128 proxy)"
x one -- python3 -c '
import socket, time
for port in (1024, 1025, 4444, 3128):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    t = time.monotonic()
    try:
        s.connect((2, port)); r = "connected: " + s.recv(64).decode().strip()
    except OSError as e:
        r = "%s errno %s" % (type(e).__name__, e.errno)
    print(port, r, "%.1f ms" % ((time.monotonic() - t) * 1000))'
echo "== row 1: relay 127.0.0.1:3128"
x one -- python3 -c 'import socket; s = socket.create_connection(("127.0.0.1", 3128)); print(s.recv(64).decode().strip())'
echo "== row 1: guest kernel config"
x one -- sh -c 'zcat /proc/config.gz | grep -E "VSOCKETS_LOOPBACK|VIRTIO_VSOCKETS="'

echo "== row 5: PID 1"; x one -- grep -E '^(Uid|Gid|Cap(Inh|Prm|Eff|Bnd|Amb)|NoNewPrivs)' /proc/1/status
echo "== row 5: exec service (parent of the exec'd shell)"
x one -- sh -c 'p=$(grep ^PPid /proc/$$/status | cut -f2); grep -E "^(Name|Uid|Gid|Groups|Cap(Inh|Prm|Eff|Bnd|Amb)|NoNewPrivs)" /proc/$p/status; cat /proc/$p/cgroup'
echo "== row 5: exec'd process"; x one -- grep -E '^(Uid|Groups|Cap(Inh|Prm|Eff|Bnd|Amb)|NoNewPrivs)' /proc/self/status
echo "== row 5: unshare -U"; x one -- unshare -U true
echo "== row 5: control: unshare -m without privilege also fails, for another reason"; x one -- unshare -m true
echo "== row 5: max_user_namespaces"; x one -- cat /proc/sys/user/max_user_namespaces
echo "== row 5: rootfs"; x one -- touch /x
echo "== row 5: setuid or setgid files"; x one -- sh -c 'find / -xdev -perm /6000 -type f | wc -l'
echo "== resolv.conf bytes"; x one -- wc -c /etc/resolv.conf

echo "== row 6: env in the box"; x one -- printenv PINFOLD_ENV_SECRET
echo "== row 6: guest cmdline"; x one -- cat /proc/cmdline
echo "== row 6: host files holding the value (none expected)"
grep -l -a -F "$PINFOLD_ENV_SECRET" "$st/one/config.json" "$st/one/console.log" "$init" ~/fc/root.ext4 "$st/work.ext4" "$st/corpus.ext4"; echo "[grep exit $? (1 = none)]"
echo "== row 6: control: the same grep finds a value written to the work disk later"

echo "== exec: exit code, signal, stdin, stderr, workdir, missing program"
x one -- sh -c 'exit 7'
x one -- sh -c 'kill -9 $$'
echo hello-stdin | x one -- cat
x one -- sh -c 'echo out; echo err >&2' 2>"$st/err"; echo "stderr: $(cat "$st/err")"
x one --workdir /work -- pwd
x one -- no-such-program
echo "== exec: tty and resize"
x one --tty --resize 40,100 -- sh -c 'sleep 0.5; tty; stty size; echo $TERM'
echo "== exec: orphan reaped"
x one -- sh -c 'true & exit 0'
x one -- sh -c 'echo "zombies: $(grep -Hs "^State:" /proc/[0-9]*/status | grep -c zombie)"'

echo "== row 11: from copy"; x one -- sh -c 'cd /work && ls -ln . sub && cat a sub/b && ./run.sh && readlink link'
echo "== row 11: read-only disk"; x one -- sh -c 'cat /corpus/c; touch /corpus/y'
echo "== row 11: ownership of the disk roots"; x one -- stat -c '%n %u:%g %a' /work /corpus

echo "== row 20: oom_score_adj of exec'd work"; x one -- cat /proc/self/oom_score_adj
echo "== row 20: stat before"; fc stat "$st/one"
echo "== row 20: memory hog"
x one -- python3 -c 'b = []
while True: b.append(bytearray(32 << 20))'
echo "== row 20: stat after"; fc stat "$st/one"
echo "== row 20: box still answers"; x one -- echo alive

echo "== row 11: hardest case: 64 MiB unsynced and a busy cwd at down"
x one -- sh -c 'head -c 64M /dev/urandom >/work/big && sha256sum /work/big >/work/big.sum && echo written'
x one -- sh -c 'cd /work && setsid sleep 1000 </dev/null >/dev/null 2>&1 & echo started'
echo "$PINFOLD_ENV_SECRET" | x one -- sh -c 'cat >/work/secret-written-by-box'
down one
echo "== row 11: needs_recovery after down (absent expected)"
dumpe2fs -h "$st/work.ext4" 2>/dev/null | grep -E '^(Filesystem features|Filesystem state)'
echo "== row 6: control: the grep finds the value the box wrote"
grep -l -a -F "$PINFOLD_ENV_SECRET" "$st/work.ext4"; echo "[grep exit $?]"
upbox two --disk "$st/work.ext4:/work:ro" --mem 256
echo "== row 11: read-only reattach"; x two -- sh -c 'cd /work && sha256sum -c big.sum; touch /work/z'
down two
echo "== exec before ENV waits for it"
upbox three --mem 256 --defer-env 1
x three -- sh -c 'echo "PATH=$PATH"' >"$st/early" 2>&1 &
early=$!
sleep 1
echo "after 1 s without ENV: $(kill -0 $early 2>/dev/null && echo still waiting || echo finished)"
fc env "$st/three"
wait $early; cat "$st/early"
down three
echo "== done"
