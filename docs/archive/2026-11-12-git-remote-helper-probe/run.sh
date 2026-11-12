#!/bin/bash
# Run every case of the 2026-11-12 git remote helper probe, in order, from
# a clean state. Usage: run.sh EMPTY_TEMP_DIR. Logs go to ./logs/.
# The box side is a second local repository behind the helper's
# box_make_bundle / box_take_bundle (PROBE_BOX=local:…, feed:… or sh:…).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$1
LOGS=$HERE/logs
mkdir -p "$LOGS"
rm -f "$LOGS"/cases.log "$LOGS"/versions.log "$LOGS"/helper-[!E]*.log
LOG=$LOGS/cases.log

export PATH="$HERE:$PATH" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
# No configuration from the calling environment either.
unset GIT_CONFIG_COUNT GIT_CONFIG_PARAMETERS $(compgen -e | grep -E '^GIT_CONFIG_(KEY|VALUE)_[0-9]+$')
export GIT_AUTHOR_NAME=probe GIT_AUTHOR_EMAIL=probe@example.invalid
export GIT_COMMITTER_NAME=probe GIT_COMMITTER_EMAIL=probe@example.invalid
export GIT_AUTHOR_DATE='2026-11-12T12:00:00Z' GIT_COMMITTER_DATE='2026-11-12T12:00:00Z'
unset PROBE_LIMIT PROBE_BRANCHES PROBE_CHECKS PROBE_PREFLIGHT PROBE_OWNREFS PROBE_LOG PROBE_LOG_CONFIG_ENV
H=$T/host
B=$T/box

{ git --version; sw_vers; python3 --version; } >"$LOGS/versions.log" 2>&1

begin() {
	CASE=$1
	echo "=== $CASE" >>"$LOG"
	export PROBE_LOG=$LOGS/helper-$CASE.log
	rm -f "$PROBE_LOG"
}
note() { echo "[$CASE] $*" >>"$LOG"; }
# One command, its output, then its own exit code.
run() {
	echo "[$CASE] \$ $*" >>"$LOG"
	"$@" >>"$LOG" 2>&1
	echo "[$CASE] exit=$?: $*" >>"$LOG"
}
show() { echo "[$CASE] --- $1" >>"$LOG"; shift; "$@" >>"$LOG" 2>&1; }

snap() {
	local f=$T/snap-$1
	{
		git -C "$H" count-objects -v
		echo "-- refs"
		git -C "$H" for-each-ref
		echo "-- object files and quarantines"
		(cd "$H/.git/objects" && find . -type f | sort && find . -type d -name 'pinfold-incoming-*')
		echo "-- fsck"
		git -C "$H" fsck --no-progress 2>&1
		echo "fsck-exit=$?"
		echo "-- .git/config"
		cat "$H/.git/config"
		echo "-- .git/hooks"
		(cd "$H/.git/hooks" 2>/dev/null && find . -type f -exec shasum {} + | sort -k2)
	} >"$f" 2>&1
}
compare() {
	if diff "$T/snap-$1" "$T/snap-$2" >"$T/diff"; then
		note "state: unchanged (count-objects, for-each-ref, object files, fsck, .git/config, .git/hooks)"
	else
		note "state: CHANGED"
		sed "s/^/[$CASE]   /" "$T/diff" >>"$LOG"
	fi
}

commit() { # repo file content message
	echo "$3" >"$1/$2"
	git -C "$1" add "$2"
	git -C "$1" commit -qm "$4"
}
fresh() {
	rm -rf "$H" "$B"
	git init -q -b main "$H"
	commit "$H" h host "host one"
	git -C "$H" remote add box pinfold::sim
	git clone -q "$H" "$B"
	git -C "$B" remote remove origin
	commit "$B" b box "box one"
	export PROBE_BOX=local:$B
	unset PROBE_LIMIT PROBE_BRANCHES PROBE_CHECKS PROBE_PREFLIGHT PROBE_OWNREFS PROBE_LOG_CONFIG_ENV
}
# The box's honest stream, as FETCH_SCRIPT prints it for a host holding nothing.
stream_of() {
	git -C "$B" for-each-ref --format='%(objectname) %(refname)' refs/heads/ >"$T/listing"
	git -C "$B" bundle create -q "$T/bundle" --branches
}
feed() { # file
	export PROBE_BOX=feed:$1
}
# A fetch from a hostile box: before, fetch, after, compare.
hostile_fetch() {
	snap before
	run git -C "$H" fetch box
	snap after
	compare before after
}

# ---------------------------------------------------------------- F: fetch
begin F0-baseline
fresh
snap before
run git -C "$H" fetch box
snap after
compare before after
show "FETCH_HEAD" cat "$H/.git/FETCH_HEAD"
show "refs/remotes/box/HEAD" git -C "$H" symbolic-ref -q refs/remotes/box/HEAD
note "box main=$(git -C "$B" rev-parse main) host refs/remotes/box/main=$(git -C "$H" rev-parse refs/remotes/box/main)"

begin F0b-incremental
commit "$B" b box2 "box two"
snap before
run git -C "$H" fetch box
snap after
compare before after
note "box main=$(git -C "$B" rev-parse main) host refs/remotes/box/main=$(git -C "$H" rev-parse refs/remotes/box/main)"

begin F1-fsck-bad-commit
fresh
tree=$(git -C "$B" write-tree)
bad=$(printf 'tree %s\nauthor noemail 1 +0000\ncommitter probe <p@e> 1 +0000\n\nbad\n' "$tree" |
	git -C "$B" hash-object -t commit -w --literally --stdin)
git -C "$B" update-ref refs/heads/bad "$bad"
note "box refs/heads/bad=$bad"
hostile_fetch

begin F2-gitmodules-update-command
fresh
printf '[submodule "x"]\n\tpath = x\n\turl = ./x\n\tupdate = !sh -c "echo F2 >>%s"\n' "$T/marker" >"$B/.gitmodules"
git -C "$B" add .gitmodules
git -C "$B" commit -qm gitmodules
hostile_fetch
show "marker" cat "$T/marker"

begin F2b-gitmodules-url-option
fresh
printf '[submodule "x"]\n\tpath = x\n\turl = --upload-pack=echo F2b >>%s\n' "$T/marker" >"$B/.gitmodules"
git -C "$B" add .gitmodules
git -C "$B" commit -qm gitmodules
hostile_fetch
show "marker" cat "$T/marker"

begin F3-dotgit-tree-entry
fresh
hook=$(printf '#!/bin/sh\necho F3 >>%s\n' "$T/marker" | git -C "$B" hash-object -w --stdin)
t1=$(printf '100755 blob %s\tpost-checkout\n' "$hook" | git -C "$B" mktree)
t2=$(printf '040000 tree %s\thooks\n' "$t1" | git -C "$B" mktree)
t3=$(printf '040000 tree %s\t.git\n' "$t2" | git -C "$B" mktree)
c=$(git -C "$B" commit-tree "$t3" -p main -m dotgit)
git -C "$B" update-ref refs/heads/dotgit "$c"
note "box refs/heads/dotgit=$c"
hostile_fetch

begin F4-truncated-bundle
fresh
stream_of
size=$(wc -c <"$T/bundle" | tr -d ' ')
head -c $((size / 2)) "$T/bundle" >"$T/half"
python3 "$HERE/craft.py" stream "$T/listing" "$T/half" "$T/feed"
note "bundle $size bytes cut to $((size / 2))"
feed "$T/feed"
hostile_fetch

begin F5-size-limit
fresh
seq 1 20000 >"$B/big"
git -C "$B" add big
git -C "$B" commit -qm big
export PROBE_LIMIT=2000
note "PROBE_LIMIT=$PROBE_LIMIT (today 4 GiB)"
hostile_fetch

begin F6a-listing-branch-count
fresh
for n in 1 2 3 4; do git -C "$B" branch "b$n"; done
export PROBE_BRANCHES=3
note "PROBE_BRANCHES=$PROBE_BRANCHES (today 4096); box has 5 branches"
hostile_fetch

begin F6b-bundle-head-count
fresh
for n in 1 2 3 4; do git -C "$B" branch "b$n"; done
stream_of
git -C "$B" for-each-ref --format='%(objectname) %(refname)' refs/heads/main >"$T/listing"
python3 "$HERE/craft.py" stream "$T/listing" "$T/bundle" "$T/feed"
feed "$T/feed"
export PROBE_BRANCHES=3
note "listing names 1 branch, bundle carries 5 heads, PROBE_BRANCHES=3"
hostile_fetch

n=0
for bad in 'refs/heads/a..b' 'refs/tags/v1' 'refs/heads/x y' 'refs/heads/x:y' 'refs/heads/-x' 'refs/heads/@{1}'; do
	n=$((n + 1))
	begin "F7.$n-listing-name"
	fresh
	stream_of
	echo "$(git -C "$B" rev-parse main) $bad" >>"$T/listing"
	python3 "$HERE/craft.py" stream "$T/listing" "$T/bundle" "$T/feed"
	feed "$T/feed"
	note "listing adds: $bad"
	hostile_fetch
done

begin F7b-bundle-head-name
fresh
stream_of
python3 "$HERE/craft.py" rename "$T/bundle" "$T/renamed" refs/heads/main 'refs/heads/x:y'
: >"$T/listing"
python3 "$HERE/craft.py" stream "$T/listing" "$T/renamed" "$T/feed"
feed "$T/feed"
note "bundle head refs/heads/main renamed to refs/heads/x:y, listing empty"
hostile_fetch

begin F7c-real-dash-branch
fresh
git -C "$B" update-ref refs/heads/-x main
note "box has refs/heads/-x"
hostile_fetch

begin F8-listed-ghost
fresh
stream_of
echo "0123456789abcdef0123456789abcdef01234567 refs/heads/ghost" >>"$T/listing"
python3 "$HERE/craft.py" stream "$T/listing" "$T/bundle" "$T/feed"
feed "$T/feed"
note "listing names refs/heads/ghost at an object neither side has"
hostile_fetch

begin F8b-bundled-head-missing
fresh
stream_of
python3 "$HERE/craft.py" rehead "$T/bundle" "$T/rehead" '# v2 git bundle' '0123456789abcdef0123456789abcdef01234567 refs/heads/ghost'
python3 "$HERE/craft.py" stream "$T/listing" "$T/rehead" "$T/feed"
feed "$T/feed"
note "bundle header names refs/heads/ghost at an object its pack lacks"
hostile_fetch

begin F9a-filtered-bundle
fresh
stream_of
note "this git's bundle create has no --filter, so the header is written by hand"
python3 "$HERE/craft.py" rehead "$T/bundle" "$T/rehead" '# v3 git bundle' '@filter=blob:none'
show "bundle header" sed -n '1,/^$/p' "$T/rehead"
python3 "$HERE/craft.py" stream "$T/listing" "$T/rehead" "$T/feed"
feed "$T/feed"
hostile_fetch

begin F9b-object-format-sha256
fresh
stream_of
python3 "$HERE/craft.py" rehead "$T/bundle" "$T/rehead" '# v3 git bundle' '@object-format=sha256'
python3 "$HERE/craft.py" stream "$T/listing" "$T/rehead" "$T/feed"
show "bundle header" sed -n '1,/^$/p' "$T/rehead"
feed "$T/feed"
hostile_fetch

begin F9c-unknown-capability
fresh
stream_of
python3 "$HERE/craft.py" rehead "$T/bundle" "$T/rehead" '# v3 git bundle' '@hooks=post-checkout'
python3 "$HERE/craft.py" stream "$T/listing" "$T/rehead" "$T/feed"
feed "$T/feed"
hostile_fetch

begin F9d-missing-prerequisite
fresh
commit "$B" b box2 "box two"
git -C "$B" for-each-ref --format='%(objectname) %(refname)' refs/heads/ >"$T/listing"
git -C "$B" bundle create -q "$T/bundle" main~1..main
show "bundle header" sed -n '1,/^$/p' "$T/bundle"
python3 "$HERE/craft.py" stream "$T/listing" "$T/bundle" "$T/feed"
feed "$T/feed"
hostile_fetch

begin F9e-long-prerequisite-subject
fresh
stream_of
held=$(git -C "$H" rev-parse main)
python3 "$HERE/craft.py" rehead "$T/bundle" "$T/rehead" '# v2 git bundle' "-$held $(head -c 100000 /dev/zero | tr '\0' x)"
python3 "$HERE/craft.py" stream "$T/listing" "$T/rehead" "$T/feed"
feed "$T/feed"
note "prerequisite $held (the host has it) with a 100000-byte subject"
hostile_fetch

# ---------------------------------------------------------------- R: refs
begin R1-df-conflict-plain
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
note "box: x deleted, x/y created, main advanced to $(git -C "$B" rev-parse main)"
hostile_fetch

begin R1b-df-conflict-atomic
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
snap before
run git -C "$H" fetch --atomic box
snap after
compare before after

begin R1c-df-conflict-preflight
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
export PROBE_PREFLIGHT=1
hostile_fetch

begin R2-lockfile-plain
fresh
run git -C "$H" fetch box
commit "$B" b box2 "box two"
git -C "$B" branch other
: >"$H/.git/refs/remotes/box/main.lock"
note "host has refs/remotes/box/main.lock; box advanced main and added other"
hostile_fetch
rm -f "$H/.git/refs/remotes/box/main.lock"

begin R2b-lockfile-atomic
fresh
run git -C "$H" fetch box
commit "$B" b box2 "box two"
git -C "$B" branch other
: >"$H/.git/refs/remotes/box/main.lock"
snap before
run git -C "$H" fetch --atomic box
snap after
compare before after
rm -f "$H/.git/refs/remotes/box/main.lock"

begin R2c-lockfile-preflight
fresh
run git -C "$H" fetch box
commit "$B" b box2 "box two"
git -C "$B" branch other
: >"$H/.git/refs/remotes/box/main.lock"
export PROBE_PREFLIGHT=1
hostile_fetch
rm -f "$H/.git/refs/remotes/box/main.lock"

# The helper commits refs/remotes/box/* itself before answering `fetch`.
begin O1-own-transaction-new-ref
fresh
export PROBE_OWNREFS=fetch
hostile_fetch

begin O2-own-transaction-fast-forward
fresh
run git -C "$H" fetch box
commit "$B" b box2 "box two"
export PROBE_OWNREFS=fetch
hostile_fetch

begin O3-own-transaction-forced
fresh
run git -C "$H" fetch box
git -C "$B" reset -q --hard HEAD~1
commit "$B" b box3 "box rewritten"
export PROBE_OWNREFS=fetch
note "box main rewritten to $(git -C "$B" rev-parse main)"
hostile_fetch

begin O4-own-transaction-df-plain
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
export PROBE_OWNREFS=fetch
hostile_fetch

begin O5-own-transaction-df-atomic
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
export PROBE_OWNREFS=fetch
snap before
run git -C "$H" fetch --atomic box
snap after
compare before after

begin O6-own-transaction-cli-refspec
fresh
export PROBE_OWNREFS=fetch
snap before
run git -C "$H" fetch box main:other
snap after
compare before after

begin R3a-cli-refspec-to-branch
fresh
snap before
run git -C "$H" fetch box main:other
snap after
compare before after

begin R3b-cli-refspec-checked-out
fresh
snap before
run git -C "$H" fetch box main:main
snap after
compare before after

begin R3c-pull
fresh
note "host main before=$(git -C "$H" rev-parse main)"
snap before
run git -C "$H" pull --no-rebase box main
snap after
compare before after
note "host main after=$(git -C "$H" rev-parse main) box main=$(git -C "$B" rev-parse main)"

begin R3d-configured-refspec
fresh
git -C "$H" config remote.box.fetch '+refs/heads/*:refs/heads/*'
hostile_fetch

begin R3e-command-line-config-refspec
fresh
export PROBE_LOG_CONFIG_ENV=1
snap before
run git -C "$H" -c 'remote.box.fetch=+refs/heads/*:refs/heads/box-*' fetch box
snap after
compare before after

begin R3f-url-not-remote
fresh
snap before
run git -C "$H" fetch pinfold::sim main:other
snap after
compare before after

begin R3g-host-hook-control
fresh
mkdir -p "$H/.git/hooks"
printf '#!/bin/sh\necho reference-transaction-$1 >>%s\n' "$T/marker" >"$H/.git/hooks/reference-transaction"
chmod +x "$H/.git/hooks/reference-transaction"
rm -f "$T/marker"
snap before
run git -C "$H" fetch box
snap after
compare before after
show "marker (the host's own hook)" cat "$T/marker"
rm -f "$H/.git/hooks/reference-transaction" "$T/marker"

# ------------------------------------------- N: what Git does with helper lines
naive() { # name, then lines appended to the honest listing
	local name=$1
	shift
	for mode in off on; do
		begin "N-$name-checks-$mode"
		fresh
		stream_of
		printf '%s\n' "$@" >>"$T/listing"
		python3 "$HERE/craft.py" stream "$T/listing" "$T/bundle" "$T/feed"
		feed "$T/feed"
		export PROBE_CHECKS=$mode
		note "listing adds: $(printf '%s ' "$@" | cut -c1-120)"
		hostile_fetch
		show "refs/remotes/box/HEAD" git -C "$H" symbolic-ref -q refs/remotes/box/HEAD
		show "config after" git -C "$H" config --local --list
	done
}
naive option-line 'option upload-pack touch-marker'
naive extra-ok 'ok refs/heads/main'
naive symref '@refs/heads/main HEAD'
naive object-format ':object-format sha256'
naive bare-dash-name "$(printf '%040d' 0 | tr 0 a) -x"
naive early-blank '' 'ok refs/heads/main'
# b7036811 is the box's main after fresh (dates are pinned).
naive long-line "b7036811f28d630ef484116a6df59f77f15c520f refs/heads/$(head -c 5000 /dev/zero | tr '\0' x)"
naive short-oid 'b703681 refs/heads/short'

# ---------------------------------------------------------------- P: push
begin P1-push-branch
fresh
commit "$H" h host2 "host two"
note "host main=$(git -C "$H" rev-parse main)"
run git -C "$H" push box main
show "box refs" git -C "$B" for-each-ref
show "host refs" git -C "$H" for-each-ref
run git -C "$H" fetch box
show "host refs after the next fetch" git -C "$H" for-each-ref

begin P2-push-to-tag
fresh
run git -C "$H" push box main:refs/tags/v1
show "box refs" git -C "$B" for-each-ref

begin P3-push-delete
fresh
run git -C "$H" push box :refs/heads/main
show "box refs" git -C "$B" for-each-ref

begin P4-box-refuses
fresh
export PROBE_BOX=local:$T/not-a-repo
run git -C "$H" push box main
show "host refs" git -C "$H" for-each-ref

begin P5-box-prints-status-lines
fresh
printf 'ok refs/heads/forged\nerror refs/heads/main lies\n' >"$T/pushfeed"
for mode in off on; do
	export PROBE_BOX=feed:$T/pushfeed PROBE_CHECKS=$mode
	note "PROBE_CHECKS=$mode: the box prints two status lines"
	run git -C "$H" push box main
	show "host refs" git -C "$H" for-each-ref
done

begin P6-push-atomic-and-force
fresh
run git -C "$H" push --atomic box main
run git -C "$H" push --force box main
run git -C "$H" push --dry-run box main

# ---------------------------------------------------------------- C: connect
begin C1-connect-fsck-bad
fresh
tree=$(git -C "$B" write-tree)
bad=$(printf 'tree %s\nauthor noemail 1 +0000\ncommitter probe <p@e> 1 +0000\n\nbad\n' "$tree" |
	git -C "$B" hash-object -t commit -w --literally --stdin)
git -C "$B" update-ref refs/heads/bad "$bad"
git -C "$H" remote add conn pinfoldconnect::x
export PROBE_CONNECT_REPO=$B
snap before
run git -C "$H" fetch conn
snap after
compare before after
note "bad commit $bad on the host: $(git -C "$H" cat-file -t "$bad" 2>&1)"

begin C2-connect-fsck-bad-fsckobjects
fresh
git -C "$B" update-ref refs/heads/bad "$(printf 'tree %s\nauthor noemail 1 +0000\ncommitter probe <p@e> 1 +0000\n\nbad\n' "$(git -C "$B" write-tree)" |
	git -C "$B" hash-object -t commit -w --literally --stdin)"
git -C "$H" remote add conn pinfoldconnect::x
snap before
run git -C "$H" -c fetch.fsckObjects=true fetch conn
snap after
compare before after

begin C3-connect-dotgit
fresh
hook=$(printf '#!/bin/sh\necho C3 >>%s\n' "$T/marker" | git -C "$B" hash-object -w --stdin)
t1=$(printf '100755 blob %s\tpost-checkout\n' "$hook" | git -C "$B" mktree)
t2=$(printf '040000 tree %s\thooks\n' "$t1" | git -C "$B" mktree)
t3=$(printf '040000 tree %s\t.git\n' "$t2" | git -C "$B" mktree)
git -C "$B" update-ref refs/heads/dotgit "$(git -C "$B" commit-tree "$t3" -p main -m dotgit)"
git -C "$H" remote add conn pinfoldconnect::x
snap before
run git -C "$H" fetch conn
snap after
compare before after

# ------------------------------------- G: no `fetch` command, refs Git writes
# Host holds refs/remotes/box/x. The box then renames x to x/y and adds z,
# both at commits the host already has, so no object is new.
no_new_objects() {
	fresh
	git -C "$B" branch x
	run git -C "$H" fetch box
	git -C "$B" branch -m x x/y
	git -C "$B" branch z main
	note "box: x renamed to x/y, z added at main; no new object"
}
begin G1-no-fetch-plain
no_new_objects
hostile_fetch

begin G1b-no-fetch-atomic
no_new_objects
snap before
run git -C "$H" fetch --atomic box
snap after
compare before after

begin G2-no-fetch-ownrefs-at-fetch
no_new_objects
export PROBE_OWNREFS=fetch
hostile_fetch

# The helper writes every ref, so Git must write none. A glob refspec with
# no destination is invalid, and with no refspec Git fetches HEAD.
begin G2b-glob-refspec-without-destination
no_new_objects
git -C "$H" config --replace-all remote.box.fetch 'refs/heads/*'
export PROBE_OWNREFS=list
hostile_fetch

begin G2c-no-refspec
no_new_objects
git -C "$H" config --unset-all remote.box.fetch
export PROBE_OWNREFS=list
hostile_fetch

begin G2d-no-refspec-new-branch
fresh
git -C "$B" branch other
git -C "$H" config --unset-all remote.box.fetch
export PROBE_OWNREFS=list
note "box: main and other, both new to the host; no fetch refspec"
hostile_fetch

# A negative refspec alone: Git wants no ref and writes none.
no_dst() { git -C "$H" config --replace-all remote.box.fetch '^refs/heads/*'; }
begin G3-ownrefs-list-df
no_new_objects
no_dst
export PROBE_OWNREFS=list
hostile_fetch

begin G3b-ownrefs-list-df-new-object
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
no_dst
export PROBE_OWNREFS=list
note "box: x deleted, x/y created, main advanced (a new object)"
hostile_fetch

begin G4-ownrefs-list-fast-forward-and-new
fresh
run git -C "$H" fetch box
commit "$B" b box2 "box two"
git -C "$B" branch other
no_dst
export PROBE_OWNREFS=list
note "box: main advanced to $(git -C "$B" rev-parse main), other added"
hostile_fetch

begin G4b-ownrefs-list-rename-no-new-object
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -m x w
no_dst
export PROBE_OWNREFS=list
note "box: x renamed to w; no new object"
hostile_fetch

begin G5-ownrefs-list-cli-refspec
fresh
no_dst
export PROBE_OWNREFS=list
snap before
run git -C "$H" fetch box main:other
snap after
compare before after

begin G6-ownrefs-list-ls-remote
fresh
no_dst
export PROBE_OWNREFS=list
snap before
run git -C "$H" ls-remote box
snap after
compare before after

begin G6b-ownrefs-list-dry-run
fresh
no_dst
export PROBE_OWNREFS=list
snap before
run git -C "$H" fetch --dry-run box
snap after
compare before after

begin G7-ownrefs-end-fast-forward
fresh
run git -C "$H" fetch box
commit "$B" b box2 "box two"
no_dst
export PROBE_OWNREFS=end
hostile_fetch

begin G8-ownrefs-end-df
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
no_dst
export PROBE_OWNREFS=end
note "box: x deleted, x/y created, main advanced (a new object)"
hostile_fetch

begin G8b-ownrefs-end-df-no-new-object
no_new_objects
no_dst
export PROBE_OWNREFS=end
hostile_fetch

begin G9-ownrefs-end-ls-remote
fresh
no_dst
export PROBE_OWNREFS=end
snap before
run git -C "$H" ls-remote box
snap after
compare before after

# ------------------------------------------------- K: the reach rule
# The host holds a commit on a host-only branch, made after the box was
# cloned, so refs/remotes/box/* cannot reach it.
reach_setup() {
	fresh
	run git -C "$H" fetch box
	git -C "$H" checkout -q -b side
	commit "$H" s side "host side"
	git -C "$H" checkout -q main
	git -C "$B" for-each-ref --format='%(objectname) %(refname)' refs/heads/ >"$T/listing"
	: >"$T/empty"
}
begin K1-reach-host-only-commit
reach_setup
echo "$(git -C "$H" rev-parse side) refs/heads/stolen" >>"$T/listing"
python3 "$HERE/craft.py" stream "$T/listing" "$T/empty" "$T/feed"
feed "$T/feed"
note "listing adds refs/heads/stolen at host-only $(git -C "$H" rev-parse side); no bundle"
hostile_fetch

begin K1b-reach-positive-control
reach_setup
echo "$(git -C "$B" rev-parse main) refs/heads/alias" >>"$T/listing"
python3 "$HERE/craft.py" stream "$T/listing" "$T/empty" "$T/feed"
feed "$T/feed"
note "listing adds refs/heads/alias at the box's main, which refs/remotes/box/main reaches; no bundle"
hostile_fetch

# ------------------------------------------- M: connectivity inside the pack
# A pack of the listed objects only, under a hand-written v2 header.
pack_bundle() { # tip, then the objects to pack
	local tip=$1
	shift
	printf '%s\n' "$@" | git -C "$B" pack-objects -q --stdout >"$T/pack"
	{ printf '# v2 git bundle\n%s refs/heads/main\n\n' "$tip"; cat "$T/pack"; } >"$T/bundle"
	echo "$tip refs/heads/main" >"$T/listing"
	python3 "$HERE/craft.py" stream "$T/listing" "$T/bundle" "$T/feed"
	feed "$T/feed"
}
pack_case() { # name, then what to build: parent | tree | blob
	for mode in on off; do
		begin "$1-checks-$mode"
		fresh
		commit "$B" b box2 "box two"
		local c2 c1
		c2=$(git -C "$B" rev-parse main)
		c1=$(git -C "$B" rev-parse main~1)
		case $2 in
		parent) pack_bundle "$c2" "$c2" "$(git -C "$B" rev-parse "$c2^{tree}")" "$(git -C "$B" rev-parse "$c2:b")"
			note "pack: box two, its tree and blob b; lacks parent $c1" ;;
		tree) pack_bundle "$c1" "$c1" "$(git -C "$B" rev-parse "$c1:b")"
			note "pack: box one and blob b; lacks tree $(git -C "$B" rev-parse "$c1^{tree}")" ;;
		blob) pack_bundle "$c1" "$c1" "$(git -C "$B" rev-parse "$c1^{tree}")"
			note "pack: box one and its tree; lacks blob $(git -C "$B" rev-parse "$c1:b")" ;;
		esac
		export PROBE_CHECKS=$mode
		hostile_fetch
	done
}
pack_case M1-pack-missing-parent parent
pack_case M2-pack-missing-tree tree
pack_case M3-pack-missing-blob blob

# ---------------------------------------------------- L: the read-cap kill
begin L1-kill-past-cap
fresh
python3 -c 'import random, sys; random.seed(1); sys.stdout.buffer.write(random.randbytes(300000))' >"$B/noise"
git -C "$B" add noise
git -C "$B" commit -qm noise
export PROBE_LIMIT=2000 PROBE_BRANCHES=3
note "PROBE_LIMIT=2000 PROBE_BRANCHES=3: the helper reads at most 2000+1+4096*4 = 18385 bytes; the box's bundle holds 300000 random bytes"
hostile_fetch
note "bundle writers still running after the helper exited: $(pgrep -f "$B -c core.warnAmbiguousRefs=false bundle create" | wc -l | tr -d ' ')"

# ---------------------------------------------- X: control bytes from the box
# Output goes to a file, then through cat -v; the raw ESC and CR counts are
# of the bytes the terminal would have received.
runv() {
	echo "[$CASE] \$ $*" >>"$LOG"
	"$@" >"$T/out" 2>&1
	local code=$?
	cat -v "$T/out" >>"$LOG"
	note "raw ESC bytes: $(LC_ALL=C tr -dc '\033' <"$T/out" | wc -c | tr -d ' '), raw CR bytes: $(LC_ALL=C tr -dc '\015' <"$T/out" | wc -c | tr -d ' ')"
	echo "[$CASE] exit=$code: $*" >>"$LOG"
}
hostile_fetch_v() {
	snap before
	runv git -C "$H" fetch box
	snap after
	compare before after
}
ESC=$(printf '\033')
CR=$(printf '\r')
begin X1-esc-bundle-capability
fresh
stream_of
python3 "$HERE/craft.py" rehead "$T/bundle" "$T/rehead" '# v3 git bundle' "@x${ESC}[31mRED${CR}cap"
python3 "$HERE/craft.py" stream "$T/listing" "$T/rehead" "$T/feed"
feed "$T/feed"
note "bundle capability line: @x ESC [31mRED CR cap"
hostile_fetch_v

begin X2-esc-bundle-header-line
fresh
stream_of
python3 "$HERE/craft.py" rehead "$T/bundle" "$T/rehead" '# v2 git bundle' "${ESC}[31mnot-a-header${CR}"
python3 "$HERE/craft.py" stream "$T/listing" "$T/rehead" "$T/feed"
feed "$T/feed"
note "bundle header line: ESC [31mnot-a-header CR"
hostile_fetch_v

for mode in on off; do
	begin "X3-esc-listed-name-checks-$mode"
	fresh
	stream_of
	echo "$(git -C "$B" rev-parse main) refs/heads/a${ESC}[31mb${CR}" >>"$T/listing"
	python3 "$HERE/craft.py" stream "$T/listing" "$T/bundle" "$T/feed"
	feed "$T/feed"
	export PROBE_CHECKS=$mode
	note "listing adds refs/heads/a ESC [31mb CR"
	hostile_fetch_v
done

begin X4-esc-gitmodules
fresh
printf '[submodule "x%s[31m"]\n\tpath = x\n\turl = --x%s[31mRED%s\n' "$ESC" "$ESC" "$CR" >"$B/.gitmodules"
git -C "$B" add .gitmodules
git -C "$B" commit -qm gitmodules
note ".gitmodules: submodule name and url carry ESC, the url a CR"
hostile_fetch_v

# -------------------------------------------------- H: hooks the box could set
begin H1-host-hooks-box-hook-file
fresh
mkdir -p "$H/.git/hooks" "$B/hooks"
for h in reference-transaction post-merge; do
	printf '#!/bin/sh\necho HOST-%s-$1 >>%s\n' "$h" "$T/marker" >"$H/.git/hooks/$h"
	chmod +x "$H/.git/hooks/$h"
done
printf '#!/bin/sh\necho BOX-post-merge >>%s\n' "$T/marker" >"$B/hooks/post-merge"
chmod +x "$B/hooks/post-merge"
git -C "$B" add hooks
git -C "$B" commit -qm "a hook file"
note "host hooks: reference-transaction, post-merge (HOST- lines); box commits hooks/post-merge (BOX- line)"
hostile_fetch
show "marker after fetch" cat "$T/marker"
snap before
run git -C "$H" pull --no-rebase box main
snap after
compare before after
show "marker after pull" cat "$T/marker"
rm -f "$T/marker"

begin H2-dotgit-tree-checks-off
fresh
hook=$(printf '#!/bin/sh\necho H2 >>%s\n' "$T/marker" | git -C "$B" hash-object -w --stdin)
t1=$(printf '100755 blob %s\tpost-checkout\n' "$hook" | git -C "$B" mktree)
t2=$(printf '040000 tree %s\thooks\n' "$t1" | git -C "$B" mktree)
t3=$(printf '040000 tree %s\t.git\n' "$t2" | git -C "$B" mktree)
git -C "$B" update-ref refs/heads/dotgit "$(git -C "$B" commit-tree "$t3" -p main -m dotgit)"
export PROBE_CHECKS=off
hostile_fetch
show "marker" cat "$T/marker"

# ------------------------------------------------ Q: prune, clone, HEAD name
prune_setup() {
	fresh
	git -C "$B" branch gone
	run git -C "$H" fetch box
	git -C "$B" branch -D gone
	note "host has refs/remotes/box/gone; the box deleted gone"
}
begin Q1-prune-flag
prune_setup
snap before
run git -C "$H" fetch --prune box
snap after
compare before after

begin Q1b-no-prune
prune_setup
hostile_fetch

begin Q2-prune-config
prune_setup
snap before
run git -C "$H" -c fetch.prune=true fetch box
snap after
compare before after

begin Q3-clone
fresh
git -C "$B" branch other
run git clone pinfold::sim "$T/clone"
show "clone refs" git -C "$T/clone" for-each-ref
show "clone HEAD" git -C "$T/clone" symbolic-ref HEAD
show "clone status" git -C "$T/clone" status --short --branch
show "clone files" ls -A "$T/clone"
rm -rf "$T/clone"

begin Q4-branch-named-HEAD
fresh
git -C "$B" checkout -q -b other
commit "$B" o other "box other"
git -C "$B" checkout -q main
git -C "$B" update-ref refs/heads/HEAD other
note "box main=$(git -C "$B" rev-parse main); refs/heads/HEAD at other=$(git -C "$B" rev-parse other)"
hostile_fetch
show "refs/remotes/box/HEAD as symref" git -C "$H" symbolic-ref -q refs/remotes/box/HEAD
show "rev-parse box" git -C "$H" rev-parse box

# ------------------------------------------------------ the URL part, config
begin R3h-url-part-not-plain
fresh
for url in 'pinfold::../x' 'pinfold::-x'; do
	git -C "$H" remote add bad "$url"
	snap before
	run git -C "$H" fetch bad
	snap after
	compare before after
	git -C "$H" remote remove bad
done

begin D1-fetch-config-names
show "git help -c, fetch.*" sh -c 'git help -c | grep "^fetch\."'

# ------------------------------- S: a box that does not read the held list
# The host holds 2000 refs under refs/remotes/box/, so the held list (82000
# bytes) is larger than a pipe buffer.
many_held() {
	fresh
	local oid
	oid=$(git -C "$H" rev-parse main)
	for i in $(seq 1 2000); do echo "create refs/remotes/box/r$i $oid"; done | git -C "$H" update-ref --stdin
}
begin S1-box-exits-unread
many_held
export PROBE_BOX=local:$T/not-a-repo
hostile_fetch

# Last: the helper may block. Git gets SIGALRM after 4 s; the box exits
# after 8 s.
begin S2-box-never-reads
many_held
export PROBE_BOX='sh:exec sleep 8'
snap before
echo "[$CASE] \$ perl -e 'alarm 4; exec @ARGV' git -C $H fetch box" >>"$LOG"
start=$(date +%s)
perl -e 'alarm 4; exec @ARGV' git -C "$H" fetch box >"$T/out" 2>&1
code=$?
note "git returned after $(($(date +%s) - start)) s"
for _ in $(seq 1 20); do pgrep -f 'git-remote-pinfold box sim' >/dev/null || break; sleep 1; done
note "helper gone $(($(date +%s) - start)) s after the start"
cat "$T/out" >>"$LOG"
echo "[$CASE] exit=$code: perl -e 'alarm 4; exec @ARGV' git -C $H fetch box" >>"$LOG"
snap after
compare before after

echo "=== done" >>"$LOG"
