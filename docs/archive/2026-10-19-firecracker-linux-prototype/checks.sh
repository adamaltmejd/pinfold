#!/bin/bash
# The existing controls, checked against a Firecracker box. Needs the proxy
# donor (env.sh proxy_up fc-proxy) running and build-rootfs.sh done.
# Prints one "CHECK name: ok|FAIL detail" line per control.
set -u
cd "$(dirname "$0")"
. ./env.sh
PROXY_SOCK=$1 EGRESS_LOG=$2
fx=$HOME/proto/fixture
marker=$HOME/proto/out/PWNED
rm -rf "$fx" "$marker"
git init -q -b main "$fx"
git -C "$fx" -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
echo hello >"$fx/README" && git -C "$fx" add README
git -C "$fx" -c user.name=t -c user.email=t@t commit -q -m readme
ok() { echo "CHECK $1: ok ${2:-}"; }
bad() { echo "CHECK $1: FAIL ${2:-}"; }
x() { "$FCBOX" exec t1 -- "$@"; }

$FCBOX up t1 "$fx" "$PROXY_SOCK"
echo "mode: FC_JAIL=${FC_JAIL:-1}"

# Kernel.
h=$(uname -r) g=$(x uname -r)
[ "$h" != "$g" ] && ok kernel "host=$h guest=$g" || bad kernel "$h $g"

# No network interface but loopback; no route anywhere.
links=$(x ip -o link | awk -F': ' '{print $2}' | tr '\n' ' ')
[ "$links" = "lo " ] && ok no-nic "links=$links" || bad no-nic "$links"
x curl -sS --noproxy '*' -m 5 https://api.github.com/ >/dev/null 2>"$OUT/direct.err"
rc=$?
[ $rc -ne 0 ] && ok direct-refused "curl exit=$rc $(head -c 120 "$OUT/direct.err")" || bad direct-refused "rc=0"

# Egress through pinfold's proxy: positive control, then refusal + log line.
n0=$(wc -l <"$EGRESS_LOG")
code=$(x curl -sS -o /dev/null -w '%{http_code}' https://api.github.com/zen)
n1=$(wc -l <"$EGRESS_LOG")
tail -n +"$((n0 + 1))" "$EGRESS_LOG" | grep -q '"decision":"allowed","host":"api.github.com"' && [ "$code" != 000 ] \
  && ok egress-allowed "http=$code" || bad egress-allowed "http=$code"
n0=$n1
x curl -sS -o /dev/null https://example.com/ 2>"$OUT/deny.err"
rc=$?
log=$(tail -n +"$((n0 + 1))" "$EGRESS_LOG")
echo "$log" | grep -q '"host":"example.com"' && [ $rc -ne 0 ] \
  && ok egress-denied "curl exit=$rc $(cat "$OUT/deny.err" | head -c 120)" || bad egress-denied "rc=$rc"
echo "$log" | sed 's/^/  log: /'

# Exec: separate streams and the exit code.
x sh -c 'echo out; echo err >&2; exit 7' >"$OUT/e.out" 2>"$OUT/e.err"
rc=$?
[ $rc = 7 ] && [ "$(cat "$OUT/e.out")" = out ] && [ "$(cat "$OUT/e.err")" = err ] \
  && ok exec-streams "rc=$rc" || bad exec-streams "rc=$rc"
[ "$(x id -u)" = 1000 ] && ok exec-uid "uid=1000" || bad exec-uid
cap=$(x grep CapBnd /proc/self/status | awk '{print $2}')
[ "$cap" = 0000000000000000 ] && ok cap-bounding "CapBnd=$cap" || bad cap-bounding "$cap"
x touch /etc/probe 2>/dev/null && bad rootfs-ro || ok rootfs-ro
echo piped | x cat | grep -q piped && ok exec-stdin || bad exec-stdin

# Cancellation: killing the host client kills the guest process group.
# A simple command in the background, so $! is pf-agent itself.
"$FCBOX" exec t1 -- sh -c 'sleep 300 & sleep 301' &
c=$!
until x pgrep -u 1000 -x sleep >/dev/null; do :; done
kill -KILL $c; wait $c 2>/dev/null
left=$(x pgrep -u 1000 -c -x sleep)
[ "$left" = 0 ] && ok cancel "sleepers=$left" || bad cancel "sleepers=$left"

# TTY attach.
printf 'tty; echo $((6*7)); exit\n' | script -qec "$FCBOX attach t1" /dev/null >"$OUT/tty.out" 2>&1
grep -q '/dev/pts/' "$OUT/tty.out" && grep -q 42 "$OUT/tty.out" \
  && ok tty-attach "$(grep -a -m1 /dev/pts "$OUT/tty.out" | tr -d '\r')" || bad tty-attach

# Host checkout: nothing of the host is mounted; writes land on the box disk.
x sh -c 'echo guest >/workspace/GUEST_WROTE; echo changed >/workspace/README'
[ ! -e "$fx/GUEST_WROTE" ] && [ "$(cat "$fx/README")" = hello ] \
  && ok host-checkout-untouched || bad host-checkout-untouched
x sh -c 'grep -E " /workspace | / " /proc/mounts' | sed 's/^/  mount: /'

# Push-back: a bundle out over exec's stdout; hooks and config stay behind.
x sh -c '
  cd /workspace
  for h in post-checkout post-merge reference-transaction pre-push post-rewrite; do
    printf "#!/bin/sh\ntouch %s\n" "'"$marker"'" >.git/hooks/$h; chmod +x .git/hooks/$h
  done
  mkdir -p .githooks .husky
  cp .git/hooks/post-checkout .githooks/post-checkout
  cp .git/hooks/post-checkout .husky/pre-commit
  git config core.hooksPath .githooks
  git config core.fsmonitor "touch '"$marker"'"
  git add -A && git -c user.name=box -c user.email=box@box commit -q -m "box work"'
x git -C /workspace bundle create - --all >"$OUT/box.bundle" 2>/dev/null
git -C "$fx" bundle verify -q "$OUT/box.bundle" 2>/dev/null
git -C "$fx" fetch -q "$OUT/box.bundle" 'refs/heads/*:refs/remotes/box/*'
git -C "$fx" merge -q --ff-only box/main
git -C "$fx" checkout -q -b after && git -C "$fx" checkout -q main
[ ! -e "$marker" ] && [ "$(cat "$fx/GUEST_WROTE")" = guest ] \
  && ok pushback-no-hooks "fetched $(git -C "$fx" log -1 --format=%s)" || bad pushback-no-hooks
# Positive control: the same hook does fire when it is the host's own.
cp "$fx/.githooks/post-checkout" "$fx/.git/hooks/post-checkout"
git -C "$fx" checkout -q after
[ -e "$marker" ] && ok hook-marker-control || bad hook-marker-control
rm -f "$fx/.git/hooks/post-checkout" "$marker"

# Teardown leaves nothing.
$FCBOX down t1
left=$(pgrep -c firecracker; true)
ls -d /srv/jailer/firecracker/t1 "$HOME/.local/state/fcbox/t1" 2>/dev/null && bad teardown-dir
grep -q -E '/srv/jailer|fcbox' /proc/mounts && bad teardown-mount
pgrep -f '[p]f-agent' && bad teardown-agent
[ "$left" = 0 ] && [ ! -e /sys/fs/cgroup/firecracker/t1 ] \
  && ok teardown "firecracker procs=$left" || bad teardown "procs=$left"
