#!/bin/bash
# Host action -> fresh guest observation. Usage: probe-h2g.sh ITER
N=$(cat /private/tmp/claude-501/-Users-adam--claude-spares-spare-WgmH1f/409584a9-d4bf-4c5c-9e08-723dcb3be88b/scratchpad/share-probe/name)
R=/private/tmp/claude-501/-Users-adam--claude-spares-spare-WgmH1f/409584a9-d4bf-4c5c-9e08-723dcb3be88b/scratchpad/share-probe/repo
i=$1
H=$R/p/h$i; G=/work/p/h$i
mkdir -p "$H/d"
printf 'one\n' > "$H/mod"; printf 'del\n' > "$H/gone"; printf 'ren\n' > "$H/ren"
printf 'atomic old\n' > "$H/atom"; printf 'inner\n' > "$H/d/f"; printf 'x\n' > "$H/exe"
g() { out=$(container exec "$N" sh -c "$1" 2>&1); echo "  guest[$1] rc=$? :: $(echo "$out" | tr '\n' '|')"; }
hi() { echo "  host: $(stat -f '%i %z %Sp' "$1" 2>&1) :: $(cat "$1" 2>&1 | tr '\n' '|')"; }

echo "== iter $i"
echo "-- modify in place"
g "cat $G/mod"
printf 'one modified and longer\n' > "$H/mod"; hi "$H/mod"
g "cat $G/mod"

echo "-- create"
g "cat $G/new"
printf 'created\n' > "$H/new"; hi "$H/new"
g "cat $G/new"

echo "-- delete"
g "cat $G/gone"
rm "$H/gone"; echo "  host: exists=$([ -e "$H/gone" ] && echo y || echo n)"
g "stat -c %i $G/gone"; g "cat $G/gone"

echo "-- rename"
g "cat $G/ren"; g "cat $G/ren2"
mv "$H/ren" "$H/ren2"; echo "  host old exists=$([ -e "$H/ren" ] && echo y || echo n)"; hi "$H/ren2"
g "cat $G/ren"; g "cat $G/ren2"

echo "-- atomic replace"
g "cat $G/atom"
printf 'atomic replacement\n' > "$H/atom.tmp"; mv -f "$H/atom.tmp" "$H/atom"; hi "$H/atom"
g "cat $G/atom"

echo "-- directory rename"
g "cat $G/d/f"; g "cat $G/d2/f"
mv "$H/d" "$H/d2"; echo "  host old exists=$([ -e "$H/d" ] && echo y || echo n)"; hi "$H/d2/f"
g "cat $G/d/f"; g "cat $G/d2/f"

echo "-- chmod +x"
g "stat -c %a $G/exe"
chmod +x "$H/exe"; hi "$H/exe"
g "stat -c %a $G/exe"

echo "-- symlink"
g "readlink $G/link"; g "cat $G/link"
ln -s mod "$H/link"; echo "  host: $(readlink "$H/link")"
g "readlink $G/link"; g "cat $G/link"
