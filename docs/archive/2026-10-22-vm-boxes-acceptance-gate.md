# vm-boxes caller-workflow acceptance gate (2026-10-22)

The gate from ARCHITECTURE.md's Open questions, issue #142, run on Linux
x64 with Firecracker before `vm-boxes` merges to main.

**Result: pass.** Two full runs, 37 of 37 checks passed in each. No host
edit and no box commit was lost. Both conflicting merges stopped without
touching the host's work.

## Setup

- Host: the homelab trial VM (labvm), a fresh lease. Debian 13, kernel
  6.12.107, 2 vCPUs (Intel i5-9500T, nested KVM), 4 GB RAM.
- pinfold: `vm-boxes` at 0c4a21a, release build,
  `cargo build -p pinfold --release --locked --target x86_64-unknown-linux-musl`
  with the toolchain pinned by rust-toolchain.toml (1.99.1). The active
  toolchain was not captured in the log.
  It reports version 0.2.7.
- Runtime: Firecracker 1.17.0 and guest kernel 6.18.57, fetched by
  pinfold on the first `up` (crates/pinfold/vmm.toml).
- Podman 6.1.4 built from the pinned commit 464c15f, as
  `scripts/setup-linux-runner.sh` does, for image builds and the root disk
  export. Debian's own podman (5.4.2) is below pinfold's minimum.
- `/dev/kvm` opened to the user with CI's udev rule. Debian has no
  `kernel.apparmor_restrict_unprivileged_userns`, so no AppArmor profile
  was loaded. Landlock was in the active LSM list.
- `pinfold doctor`: Firecracker preflight ok.
- The caller surface only: a JSON box spec on `box up`'s stdin, `box exec`,
  `box down`, `box fetch`, `disk list`, `disk rm` and `image build`, plus
  plain git on the host. The full script is in the appendix.

## Scenario

The project is a dependency-free Rust crate `gatecalc` in a fresh git repo
on the host (`~/proj`), with one commit (`add()` and its test).

1. **Dirty and staged host work.** `sub()` is appended to `src/lib.rs`
   and staged. A line is appended to `README.md` and left unstaged.
   `notes.txt` is untracked. The host's HEAD, `git diff --cached`,
   `git diff` and file hashes are recorded.
2. **Caller image.** `pinfold image build gate` from
   `FROM debian:trixie-slim` plus `git ca-certificates cargo rustc`.
3. **Box spec.** Two disks: `gate-work` at `/work` with `from` set to the
   host project (4G), and `gate-home` at `/home/box` (2G). `env` sets
   `HOME`, `CARGO_HOME`, `CARGO_NET_OFFLINE` and a git identity, and
   `safe.directory=*` through `GIT_CONFIG_*`. No egress. 2 CPUs, 2G.
4. **up 1.** `box up` with the spec on a FIFO held open. In the box,
   `git diff --cached` and `git status --porcelain` are compared with the
   host's.
5. **Work 1.** In the box: commit the staged `sub()`. Append `mul()` and a
   test module, and rewrite `add()`'s doc line. `cargo build`, then
   `cargo test` (2 tests pass). Commit `src/lib.rs` only, so the dirty
   README stays uncommitted in the box. Write `$HOME/state.txt`. Meanwhile
   the host appends a second line to `README.md`.
6. **down and up 2.** `box down gate`, then `box up` with the same spec.
   The disks are reused and `from` is ignored.
7. **Work 2.** Append `div()`, `cargo test`, commit.
8. **Fetch for review.**
   `pinfold box fetch gate --repo /work --into ~/proj`, then
   `git log main..gate/main` and `git diff --stat main...gate/main`.
9. **Merge 1, with the host editing the conflicting file.** The host
   appends a line to `src/lib.rs`, which also still holds the staged
   `sub()`. Then `git merge --no-edit gate/main`.
10. **Merge 2, with a committed conflicting edit.** The host rewrites
    `add()`'s doc line differently and commits everything (`commit -am`).
    Then `git merge --no-edit gate/main`. The conflict is resolved by
    taking the box's file, combining both doc lines and re-appending the
    host's line, and the merge is committed. `cargo test` runs on the host.
11. **Teardown.** `box down gate`, then `disk rm` for each disk.

## Checks

Each check is the same in both runs.

| Check | What it asserts | Run 1 | Run 2 |
|---|---|---|---|
| image-built | `image build` printed `built` | pass | pass |
| probe-* (5) | each probe `up` printed `ready` | pass | pass |
| up1-ready, up2-ready | `ready` | pass | pass |
| box-staged-matches-host | the box's `git diff --cached` equals the host's | pass | pass |
| box-status-matches-host | the box's `git status --porcelain` equals the host's: staged, dirty and untracked all crossed | pass | pass |
| box-build, box-test-1, box-test-2 | `cargo build` and `cargo test` exit 0 in the box | pass | pass |
| down1, down2 | `up` exits 0 after `box down`, with a `down` line | pass | pass |
| state-head | the box HEAD after `up` 2 equals the one before `down` | pass | pass |
| state-home | `$HOME/state.txt` survived on the home disk | pass | pass |
| state-untracked | the untracked `notes.txt` is still in `/work` | pass | pass |
| state-target | `target/debug` survived | pass | pass |
| state-from-ignored | the host's later README edit did not reach the box | pass | pass |
| fetched, fetch-ref | `fetched` line; `refs/remotes/gate/main` is the box HEAD | pass | pass |
| fetch-host-untouched | the host's HEAD, index, working diff and file hashes are unchanged by the fetch | pass | pass |
| review-three-commits | `main..gate/main` holds the three box commits | pass | pass |
| merge1-stops | `git merge` exits non-zero | pass | pass |
| merge1-no-overwrite | the host's HEAD, index, working diff and file hashes are unchanged | pass | pass |
| merge1-no-merge-head | no merge is left in progress | pass | pass |
| merge2-stops, merge2-conflict-marked, merge2-head-unmoved | the merge exits non-zero, leaves conflict markers and does not move HEAD | pass | pass |
| host-test | `cargo test` passes on the merged host tree | pass | pass |
| reachable-* (4) | both box commits, the host's first commit and its conflicting commit are ancestors of the final HEAD | pass | pass |
| host-edits-kept | both README lines, the during-merge line and `notes.txt` are in the final tree | pass | pass |
| box-edits-kept | `sub`, `mul` and `div` are in the final tree | pass | pass |

Merge 1's refusal was git's own:
`error: Your local changes to the following files would be overwritten by merge: src/lib.rs ... Aborting`.
pinfold has no merge step. What it guarantees is that the fetch changed
only `refs/remotes/gate/*`.

## Timings

Wall time in seconds. Ready is from spawning `box up` to its first stdout
line. Teardown is from `box down` until `up` has exited, after printing its
`down` line. Whether the Firecracker process was gone at that instant was
not observed separately; the disks reporting `clean` afterwards shows the
guest had unmounted them. Work and fetch
time the `box exec` and `box fetch` calls. Run 1 started from empty
caches. Run 2 reused the image, the Firecracker and kernel artifacts, and
the image's root disk.

| Phase | Run 1 | Run 2 |
|---|---|---|
| setup: `image build` (Debian base pull and apt in run 1) | 35.09 | 1.70 |
| ready, first `up` of the image (artifact download, 970 MB root disk export) | 11.39 | 2.02 |
| ready, warm, one empty 1G disk | 2.23 | 1.92 |
| ready, warm, plus a 4.1 MB read-only mount | 2.07 | 2.03 |
| ready, `from` = 186 MB / 1,025 files (pinfold's tree with `target/`) | 3.76 | 2.72 |
| ready, the same disk again (`from` ignored) | 1.96 | 1.96 |
| ready, gate `up` 1 (`from` = the small project) | 2.03 | 2.12 |
| work 1: commit, edit, build, test, commit | 1.47 | 1.41 |
| teardown 1 | 0.08 | 0.07 |
| ready, gate `up` 2 (state reused) | 1.97 | 2.09 |
| work 2: edit, test, commit | 1.20 | 1.18 |
| fetch | 0.10 | 0.07 |
| teardown 2 | 0.08 | 0.10 |
| teardown: three `disk rm` | <0.01 | <0.01 |

Disk images are sparse. The 1G probe disk allocated 944 KB, and
`disk list` reported it `clean` after `down`.

## Reading for #128 and #130

- **#128 (read-only mounts per start).** A 4.1 MB mount added no
  measurable time: the difference was within run-to-run noise of about
  0.2 s. This agrees with 2026-10-21-mount-image-cost.md, which found
  about 0.15 s for 1.7 MB, 0.3 s for 53 MB and 1.3 s for pi's 110 MB
  harness. The gate adds nothing new for large mounts. Pi's harness stays
  the case that matters.
- **#130 (copy-on-write lane disks).** Setup does not dominate this
  workflow. The one-time costs are the image build and the first `up` of
  an image (35 s and 11 s, cold), and they are shared by every lane. The
  per-lane cost is the `from` copy: about 0.1 s for the small project and
  0.8–1.8 s for 186 MB, against a warm `ready` of about 2 s and per-phase
  work of 1–1.5 s. A lane that copies a tree of a few hundred MB pays
  about a second. These numbers do not justify cloning a template disk.
  Revisit with a multi-GB `from` (a large `target/` or `node_modules`).
  The cost is assumed to scale with bytes, as the mount copy does, but
  that is not measured here.
- `box down` returned and `up` exited in under 0.15 s every time, and
  `fetch` took 0.1 s for three commits. Neither is a cost worth tuning.

## Observations

These are not failures, and no fix was made here.

- `git status` in `/work` warns
  `could not open directory 'lost+found/': Permission denied`. The ext4
  `lost+found` sits at the disk root, so a project disk mounted at the
  repo root shows it inside the work tree. Git skips it and the status
  comparison still matched the host's, but every git command in the box
  prints the warning.
- Dirty host work that crosses through `from` can be committed in the
  box. A box `git commit -a` would commit the host's unstaged README line
  into box history. The gate's box commits name `src/lib.rs` only. If
  the README had gone in, merge 2 would also have conflicted on README.
  It would still have stopped rather than overwrite.
- `safe.directory=*` was set in the spec up front and never tested
  without, so whether a project disk needs it on Linux is open.
- Run 1's script printed only the last three lines of `cargo test`. Run 2
  printed every `test result` line: 2 unit tests passed in each box test.
  That was the only change to the script between the runs.

## Appendix: the script

It runs on labvm as the lease user and reads its inputs from the
environment shown. `~/pinfold` is `git archive` of 0c4a21a with the
release `target/`, and `~/pinfold-src` is a copy taken before the build.

```bash
#!/bin/bash
# #142 caller-workflow acceptance gate. Runs on labvm as `agent`, CLI and box spec only.
set -uo pipefail
export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
export PATH=/usr/local/bin:$PATH
PF=$HOME/.local/bin/pinfold
G=$HOME/gate
P=$HOME/proj
rm -rf "$G" "$P"; mkdir -p "$G"
RES=$G/results; TIM=$G/timings
: >"$RES"; : >"$TIM"

now() { date +%s.%N; }
lap() { awk -v n="$1" -v a="$2" -v b="$3" 'BEGIN{printf "%s %.2f\n", n, b-a}' | tee -a "$TIM"; }
check() { # name, then a command whose success is PASS
  local name=$1; shift
  if "$@"; then echo "PASS $name" | tee -a "$RES"; else echo "FAIL $name" | tee -a "$RES"; fi
}
bx() { "$PF" box exec gate --workdir /work -- "$@"; }
snap() { # host state fingerprint: HEAD, index, working diff, file contents
  git -C "$P" rev-parse HEAD
  git -C "$P" diff --cached | sha256sum
  git -C "$P" diff | sha256sum
  (cd "$P" && sha256sum README.md src/lib.rs notes.txt 2>/dev/null)
}

UPFD=
up() { # $1 spec file, $2 label; times spawn -> ready
  rm -f "$G/in"; mkfifo "$G/in"
  local t0; t0=$(now)
  "$PF" box up <"$G/in" >"$G/up-$2.out" 2>"$G/up-$2.err" &
  UPPID=$!
  exec {UPFD}>"$G/in"
  cat "$1" >&"$UPFD"
  until grep -q '"event"' "$G/up-$2.out" 2>/dev/null; do
    kill -0 "$UPPID" 2>/dev/null || break
    sleep 0.05
  done
  local t1; t1=$(now)
  lap "ready:$2" "$t0" "$t1"
  head -1 "$G/up-$2.out"
  grep -q '"event":"ready"' "$G/up-$2.out"
}
down() { # $1 label; times `box down` -> `up` exited
  local t0; t0=$(now)
  "$PF" box down gate
  wait "$UPPID"; local rc=$?
  exec {UPFD}>&-
  local t1; t1=$(now)
  lap "teardown:$1" "$t0" "$t1"
  tail -1 "$G/up-$1.out"
  return $rc
}

echo "== host"; uname -r; nproc; free -m | head -2; "$PF" --version 2>/dev/null; podman --version
"$PF" doctor | tee "$G/doctor.json"

# ---- host project with staged, dirty and untracked work
mkdir -p "$P/src"; cd "$P" || exit 1
git init -q -b main
git config user.name host; git config user.email host@example.invalid
printf '[package]\nname = "gatecalc"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n' >Cargo.toml
printf '# gatecalc\n' >README.md
cat >src/lib.rs <<'EOF'
/// Adds two numbers.
pub fn add(a: i64, b: i64) -> i64 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds() {
        assert_eq!(add(2, 3), 5);
    }
}
EOF
printf 'target/\n' >.gitignore
git add -A; git commit -qm "host: initial"
C0=$(git rev-parse HEAD)
# staged: sub()
cat >>src/lib.rs <<'EOF'

/// Subtracts b from a.
pub fn sub(a: i64, b: i64) -> i64 {
    a - b
}
EOF
git add src/lib.rs
# dirty, unstaged
printf 'Host note, unstaged.\n' >>README.md
# untracked
printf 'host scratch notes\n' >notes.txt
snap >"$G/host-0"
git status --porcelain >"$G/host-status-0"
git diff --cached >"$G/host-cached-0"

# ---- setup: caller image
mkdir -p "$G/ctx"
printf 'FROM docker.io/library/debian:trixie-slim\nRUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates cargo rustc && rm -rf /var/lib/apt/lists/*\n' >"$G/Containerfile"
t0=$(now)
"$PF" image build gate --containerfile "$G/Containerfile" --context "$G/ctx" | tee "$G/build.json"
t1=$(now); lap "setup:image-build" "$t0" "$t1"
check image-built grep -q '"event":"built"' "$G/build.json"

cat >"$G/spec.json" <<EOF
{"name":"gate","image":"pinfold/image-gate:latest","cpus":2,"memory":"2G",
 "disks":[{"name":"gate-work","guest":"/work","size":"4G","from":"$P"},
          {"name":"gate-home","guest":"/home/box","size":"2G"}],
 "env":{"HOME":"/home/box","CARGO_HOME":"/home/box/.cargo","CARGO_NET_OFFLINE":"true",
        "GIT_AUTHOR_NAME":"box","GIT_AUTHOR_EMAIL":"box@example.invalid",
        "GIT_COMMITTER_NAME":"box","GIT_COMMITTER_EMAIL":"box@example.invalid",
        "GIT_CONFIG_COUNT":"1","GIT_CONFIG_KEY_0":"safe.directory","GIT_CONFIG_VALUE_0":"*"}}
EOF

# Probe: same image, one empty disk. Its first up pays the Firecracker and
# kernel download and the image's root disk; the second is the warm baseline.
printf '{"name":"gate","image":"pinfold/image-gate:latest","cpus":2,"memory":"2G","disks":[{"name":"gate-probe","guest":"/probe","size":"1G"}]}\n' >"$G/probe.json"
check probe-cold-ready up "$G/probe.json" probe-cold
down probe-cold
du -h --apparent-size ~/.local/state/pinfold/disks/gate-probe.ext4 | tee -a "$TIM"; du -h ~/.local/state/pinfold/disks/gate-probe.ext4 | tee -a "$TIM"; ls -la ~/.cache/pinfold/root/ | tee -a "$TIM"
check probe-warm-ready up "$G/probe.json" probe-warm
down probe-warm
# #128: the same warm probe plus a read-only mount (pinfold's source, no target/).
printf '{"name":"gate","image":"pinfold/image-gate:latest","cpus":2,"memory":"2G","disks":[{"name":"gate-probe","guest":"/probe","size":"1G"}],"mounts":[{"host":"%s","guest":"/ref"}]}\n' "$HOME/pinfold-src" >"$G/probe-mount.json"
du -sh "$HOME/pinfold-src" | tee -a "$TIM"
check probe-mount-ready up "$G/probe-mount.json" probe-mount
down probe-mount
# #130: a realistic `from` (pinfold's checkout with its release target/).
du -sh "$HOME/pinfold" | tee -a "$TIM"
printf '{"name":"gate","image":"pinfold/image-gate:latest","cpus":2,"memory":"2G","disks":[{"name":"gate-big","guest":"/big","size":"16G","from":"%s"}]}\n' "$HOME/pinfold" >"$G/probe-big.json"
check probe-big-ready up "$G/probe-big.json" probe-big
down probe-big
check probe-big-again up "$G/probe-big.json" probe-big-again
down probe-big-again
"$PF" disk rm gate-big

# ---- up 1: setup's `from` copy, then ready
cd "$P"
check up1-ready up "$G/spec.json" up1
check box-staged-matches-host bash -c "diff <(\"$PF\" box exec gate --workdir /work -- git diff --cached) \"$G/host-cached-0\""
check box-status-matches-host bash -c "diff <(\"$PF\" box exec gate --workdir /work -- git status --porcelain) \"$G/host-status-0\""

# ---- work 1: commit the staged work, a real edit, build and test
t0=$(now)
bx git commit -qm "box: host's staged sub()"
bx sh -c 'cat >>src/lib.rs <<EOF

/// Multiplies two numbers.
pub fn mul(a: i64, b: i64) -> i64 {
    a * b
}

#[cfg(test)]
mod mul_tests {
    #[test]
    fn muls() {
        assert_eq!(super::mul(4, 5), 20);
        assert_eq!(super::sub(4, 5), -1);
    }
}
EOF
sed -i "s|^/// Adds two numbers.|/// Adds two numbers (box).|" src/lib.rs'
bx cargo build -q; check box-build test $? = 0
bx cargo test 2>&1 | grep "test result"; check box-test-1 test "${PIPESTATUS[0]}" = 0
bx git commit -qm "box: mul() and doc" -- src/lib.rs
bx sh -c 'echo box-state >$HOME/state.txt'
t1=$(now); lap "work:1" "$t0" "$t1"
B1=$(bx git rev-parse HEAD); echo "B1=$B1"
# A host edit while the box works; it must survive everything below.
printf 'Host note 2, while the box worked.\n' >>README.md
snap >"$G/host-1"

check down1 down up1
check up2-ready up "$G/spec.json" up2

# ---- state kept across down and up
check state-head test "$(bx git rev-parse HEAD)" = "$B1"
check state-home test "$(bx cat /home/box/state.txt)" = box-state
check state-untracked test "$(bx cat notes.txt)" = "host scratch notes"
check state-target bx test -d target/debug
check state-from-ignored bash -c "! \"$PF\" box exec gate --workdir /work -- grep -q 'Host note 2' README.md"

# ---- work 2: second edit after the restart
t0=$(now)
bx sh -c 'cat >>src/lib.rs <<EOF

/// Divides a by b.
pub fn div(a: i64, b: i64) -> i64 {
    a / b
}
EOF'
bx cargo test 2>&1 | grep "test result"; check box-test-2 test "${PIPESTATUS[0]}" = 0
bx git commit -qm "box: div()" -- src/lib.rs
t1=$(now); lap "work:2" "$t0" "$t1"
B2=$(bx git rev-parse HEAD); echo "B2=$B2"

# ---- fetch for review
snap >"$G/host-pre-fetch"
t0=$(now)
"$PF" box fetch gate --repo /work --into "$P" | tee "$G/fetch.json"
t1=$(now); lap "fetch" "$t0" "$t1"
check fetched grep -q '"event":"fetched"' "$G/fetch.json"
check fetch-ref test "$(git rev-parse refs/remotes/gate/main)" = "$B2"
snap >"$G/host-post-fetch"; check fetch-host-untouched diff "$G/host-post-fetch" "$G/host-pre-fetch"
git log --oneline main..gate/main | tee "$G/review-log"
check review-three-commits test "$(git rev-list --count main..gate/main)" = 3
git diff --stat main...gate/main

# ---- merge 1: host has staged and unstaged edits to the conflicting file
printf '// host edit during merge\n' >>src/lib.rs
snap >"$G/host-pre-merge1"
git merge --no-edit gate/main >"$G/merge1.out" 2>&1; rc=$?
cat "$G/merge1.out"
check merge1-stops test "$rc" != 0
snap >"$G/host-post-merge1"; check merge1-no-overwrite diff "$G/host-post-merge1" "$G/host-pre-merge1"
check merge1-no-merge-head test ! -e .git/MERGE_HEAD

# ---- merge 2: host commits a conflicting edit, then merges
sed -i 's|^/// Adds two numbers.|/// Adds two numbers (host).|' src/lib.rs
git commit -qam "host: sub(), doc, note"
H1=$(git rev-parse HEAD)
git merge --no-edit gate/main >"$G/merge2.out" 2>&1; rc=$?
cat "$G/merge2.out"
check merge2-stops test "$rc" != 0
check merge2-conflict-marked grep -q '^<<<<<<<' src/lib.rs
check merge2-head-unmoved test "$(git rev-parse HEAD)" = "$H1"
# Resolve: keep both doc tags, take the box's code.
git checkout --theirs src/lib.rs
sed -i 's|^/// Adds two numbers (box).|/// Adds two numbers (host, box).|' src/lib.rs
printf '// host edit during merge\n' >>src/lib.rs
git add src/lib.rs; git commit -qm "host: merge box work"
check host-test bash -c 'cargo test -q >/dev/null 2>&1'
for c in "$B1" "$B2" "$C0" "$H1"; do check "reachable-${c:0:7}" git merge-base --is-ancestor "$c" HEAD; done
check host-edits-kept bash -c "grep -q 'Host note, unstaged.' README.md && grep -q 'Host note 2' README.md && grep -q 'host edit during merge' src/lib.rs && test \"\$(cat notes.txt)\" = 'host scratch notes'"
check box-edits-kept bash -c "grep -q 'pub fn mul' src/lib.rs && grep -q 'pub fn div' src/lib.rs && grep -q 'pub fn sub' src/lib.rs"

# ---- teardown
check down2 down up2
t0=$(now); "$PF" disk rm gate-work; "$PF" disk rm gate-home; "$PF" disk rm gate-probe; t1=$(now); lap "teardown:disk-rm" "$t0" "$t1"
"$PF" disk list
echo "== results"; cat "$RES"; echo "== timings"; cat "$TIM"
grep -c ^FAIL "$RES"
echo GATE_DONE
```
