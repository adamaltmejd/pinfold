#!/bin/bash
# sabotage.sh SRC [NAME]...: build each named sabotage (default all) of SRC's init into
# ~/fc/sab-NAME.ext4, then run checks.sh against it into out/sab-NAME.txt.
set -euo pipefail
src=$1
shift
only=" $* "
here=$(cd "$(dirname "$0")" && pwd)
export CARGO_TARGET_DIR=~/sab-target
mkdir -p "$here/out"
sabotage() {
  local name=$1 find=$2 replace=$3
  [ "$only" = "  " ] || [[ $only == *" $name "* ]] || return 0
  rm -rf ~/sab-src && cp -a "$src" ~/sab-src && rm -rf ~/sab-src/target
  python3 - ~/sab-src/crates/pinfold/src/init/firecracker.rs "$find" "$replace" <<'PY'
import sys
path, find, replace = sys.argv[1:]
text = open(path).read()
assert text.count(find) == 1, find
open(path, "w").write(text.replace(find, replace))
PY
  "$here/build.sh" ~/sab-src "sab-$name"
  "$here/checks.sh" "sab-$name" >"$here/out/sab-$name.txt" 2>&1 || true
}
sabotage no-cgroup-kill 'fs::write(format!("{WORK}/cgroup.kill"), "1")?;' ''
sabotage pid1-keeps-all \
  'drop_bounding(Some(CAP_SYS_ADMIN)).and_then(|()| set_capabilities(Some(CAP_SYS_ADMIN)))' \
  'Ok::<(), io::Error>(())'
sabotage service-bounding 'drop_bounding(None)?;' ''
sabotage userns-on 'fs::write("/proc/sys/user/max_user_namespaces", "0")?;' ''
sabotage no-oom-raise '|| !raise_oom_score()' ''
sabotage spawn-panics 'thread::Builder::new().spawn(work)' 'Ok(thread::spawn(work))'
sabotage guest-allows-proc '|| ["/proc", "/sys", "/dev"]' '|| ["/no-such-root"]'
