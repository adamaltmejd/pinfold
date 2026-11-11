#!/bin/bash
# Guest action (fresh exec) -> host observation. Usage: probe-g2h.sh ITER
N=$(cat /private/tmp/claude-501/-Users-adam--claude-spares-spare-WgmH1f/409584a9-d4bf-4c5c-9e08-723dcb3be88b/scratchpad/share-probe/name)
R=/private/tmp/claude-501/-Users-adam--claude-spares-spare-WgmH1f/409584a9-d4bf-4c5c-9e08-723dcb3be88b/scratchpad/share-probe/repo
i=$1
H=$R/p/g$i; G=/work/p/g$i
mkdir -p "$H/d"
printf 'one\n' > "$H/mod"; printf 'del\n' > "$H/gone"; printf 'ren\n' > "$H/ren"
printf 'atomic old\n' > "$H/atom"; printf 'inner\n' > "$H/d/f"; printf 'x\n' > "$H/exe"
g() { out=$(container exec "$N" sh -c "$1" 2>&1); echo "  guest[$1] rc=$? :: $(echo "$out" | tr '\n' '|')"; }
h() { echo "  host[$1]: $(eval "$1" 2>&1 | tr '\n' '|')"; }

echo "== iter $i"
echo "-- modify in place"
g "cat $G/mod >/dev/null; printf 'one modified and longer\n' > $G/mod"
h "stat -f '%i %z' $H/mod; cat $H/mod"
echo "-- create"
g "printf 'created\n' > $G/new"
h "stat -f '%u:%g %Sp' $H/new; cat $H/new"
echo "-- delete"
g "rm $G/gone"
h "ls $H/gone"
echo "-- rename"
g "mv $G/ren $G/ren2"
h "ls $H/ren; cat $H/ren2"
echo "-- atomic replace"
g "printf 'atomic replacement\n' > $G/atom.tmp && mv -f $G/atom.tmp $G/atom"
h "cat $H/atom; ls $H/atom.tmp"
echo "-- directory rename"
g "mv $G/d $G/d2"
h "ls $H/d; cat $H/d2/f"
echo "-- chmod +x"
g "chmod +x $G/exe"
h "stat -f '%Sp' $H/exe"
echo "-- symlink"
g "ln -s mod $G/link"
h "readlink $H/link; cat $H/link"
echo "-- guest re-read of its own changes (fresh exec)"
g "cat $G/mod $G/new $G/ren2 $G/atom $G/d2/f; readlink $G/link; stat -c %a $G/exe; ls $G/gone $G/ren $G/d"
