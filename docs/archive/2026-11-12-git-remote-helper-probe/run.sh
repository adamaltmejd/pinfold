#!/bin/bash
# Run every case of the 2026-11-12 git remote helper probe, in order, from
# a clean state. Usage: run.sh EMPTY_TEMP_DIR. Logs go to ./logs/.
# The box side is a second local repository behind the helper's
# box_make_bundle / box_take_bundle (PROBE_BOX=local:… or feed:…).
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
unset PROBE_LIMIT PROBE_BRANCHES PROBE_CHECKS PROBE_PREFLIGHT PROBE_OWNREFS PROBE_LOG
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
	} >"$f" 2>&1
}
compare() {
	if diff "$T/snap-$1" "$T/snap-$2" >"$T/diff"; then
		note "state: unchanged (count-objects, for-each-ref, object files, fsck)"
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
	unset PROBE_LIMIT PROBE_BRANCHES PROBE_CHECKS PROBE_PREFLIGHT PROBE_OWNREFS
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
export PROBE_OWNREFS=1
hostile_fetch

begin O2-own-transaction-fast-forward
fresh
run git -C "$H" fetch box
commit "$B" b box2 "box two"
export PROBE_OWNREFS=1
hostile_fetch

begin O3-own-transaction-forced
fresh
run git -C "$H" fetch box
git -C "$B" reset -q --hard HEAD~1
commit "$B" b box3 "box rewritten"
export PROBE_OWNREFS=1
note "box main rewritten to $(git -C "$B" rev-parse main)"
hostile_fetch

begin O4-own-transaction-df-plain
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
export PROBE_OWNREFS=1
hostile_fetch

begin O5-own-transaction-df-atomic
fresh
git -C "$B" branch x
run git -C "$H" fetch box
git -C "$B" branch -D x
git -C "$B" branch x/y
commit "$B" b box2 "box two"
export PROBE_OWNREFS=1
snap before
run git -C "$H" fetch --atomic box
snap after
compare before after

begin O6-own-transaction-cli-refspec
fresh
export PROBE_OWNREFS=1
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
run git -C "$H" pull --no-rebase box main
note "host main after=$(git -C "$H" rev-parse main) box main=$(git -C "$B" rev-parse main)"

begin R3d-configured-refspec
fresh
git -C "$H" config remote.box.fetch '+refs/heads/*:refs/heads/*'
hostile_fetch

begin R3e-command-line-config-refspec
fresh
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
run git -C "$H" fetch box
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
		note "listing adds: $*"
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

echo "=== done" >>"$LOG"
