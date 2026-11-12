# Shared helpers for the 2026-11-12 worktree probe. Source with T set.
# T is the temp root; R=$T/R is the main checkout. Every share is mounted
# in the box at its host path. Host Git runs with no global or system
# config, so the operator's hooks and signing stay out. Box Git runs with
# no global config only; it does not set GIT_CONFIG_NOSYSTEM.
IMAGE=localhost/pinfold/profile-default:latest
R=$T/R
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
  GIT_AUTHOR_NAME=host GIT_AUTHOR_EMAIL=h@x \
  GIT_COMMITTER_NAME=host GIT_COMMITTER_EMAIL=h@x
GPRE="export GIT_CONFIG_GLOBAL=/dev/null GIT_AUTHOR_NAME=box GIT_AUTHOR_EMAIL=b@x \
GIT_COMMITTER_NAME=box GIT_COMMITTER_EMAIL=b@x \
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0='*';"

# h DIR CMD: run CMD on the host in DIR, print it, its output and exit code.
h() {
  local d=$1; shift
  echo "host\$ (cd $d) $*"
  (cd "$d" && bash -c "$*") 2>&1 | sed 's/^/  /'
  echo "  rc=${PIPESTATUS[0]}"
}

# g CT DIR CMD: the same in box CT.
g() {
  local ct=$1 d=$2; shift 2
  echo "box[$ct]\$ (cd $d) $*"
  container exec "$ct" sh -c "$GPRE cd $d && { $* ; }" 2>&1 | sed 's/^/  /'
  echo "  rc=${PIPESTATUS[0]}"
}

# box NAME EXTRA_MOUNT_ARGS...: start a box sharing R with R/.git read-only.
box() {
  local n=$1; shift
  echo "host\$ container run -d --name $n --user 501:20 -v $R:$R -v $R/.git:$R/.git:ro $* --entrypoint sleep IMAGE infinity"
  container run -d --name "$n" --user 501:20 -v "$R:$R" -v "$R/.git:$R/.git:ro" "$@" \
    --entrypoint sleep "$IMAGE" infinity >/dev/null 2>&1
  echo "  rc=$?"
}

# fresh: a main checkout R with one commit and a linked worktree R/wt/x on branch x.
fresh() {
  mkdir -p "$R"
  h "$R" "git init -q -b main . && echo a > a && printf 'wt/\n' > .gitignore && git add a .gitignore && git commit -qm c0 && git worktree add -q wt/x -b x && cat wt/x/.git && git worktree list"
}
