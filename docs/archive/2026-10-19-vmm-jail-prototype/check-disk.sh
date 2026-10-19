#!/bin/bash
# Check 3: is the data disk clean after down? Option A (PID 1 unmounts),
# option C (child mount namespace dies), and the controls.
. ~/proto/lib.sh
WORK='cd /data && dd if=/dev/urandom of=f bs=1M count=64 status=none && sha256sum f >f.sum && echo dirty-tail >>f.sum && sha256sum f >f.sum2; (cd /data && setsid sleep 1000 </dev/null >/data/held 2>&1 &); grep " /data " /proc/self/mounts; echo written'
state() { dumpe2fs -h "$1" 2>/dev/null | grep -E '^Filesystem (state|features)'; }
trial() { # trial LABEL MODE DOWN
  local d=$ST/t.ext4; mkdata $d 256M
  echo "=== $1 (mode $2, down: $3)"
  up t "$2" $d >/dev/null || return
  gexec t "$WORK"
  case $3 in
    unmount|unmount-nokill) echo "down reply: $(gctl t $3)" ;;
    kill) : ;;
  esac
  kill_vm t
  grep -a 'pf-guest' $ST/t/console.log | sed 's/^/  console: /'
  state $d | sed 's/^/  host: /'
  up r ro $d >/dev/null
  grep -a -E 'pf-guest: mount|EXT4|ext4' $ST/r/console.log | sed 's/^/  ro console: /'
  gexec r 'grep " /data " /proc/mounts || echo "no /data mount"; cd /data 2>/dev/null && sha256sum -c f.sum2'
  kill_vm r
  state $d | sed 's/^/  host after ro: /'
}
trial "negative control: VM killed, no unmount" A kill
trial "option A without ending the work" A unmount-nokill
for i in 1 2 3; do trial "option A #$i" A unmount; done
for i in 1 2 3; do trial "option C #$i" C unmount; done
