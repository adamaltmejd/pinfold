#!/bin/bash
# Does readdir (ls) of the parent, or ls -l, heal stale lookups inside the window? Usage: readdir-heal.sh CONTAINER HOSTDIR TAG LSCMD
N=$1; H=$2/$3; G=/work/$3; L=$4; mkdir -p $H; echo old > $H/atom; echo d > $H/del; echo r > $H/ren; echo x > $H/chm; chmod 644 $H/chm
container exec $N sh -c "cat $G/atom $G/del $G/ren >/dev/null; cat $G/ren2 2>/dev/null; stat $G/chm >/dev/null"
echo new > $H/atom.tmp; mv -f $H/atom.tmp $H/atom; rm $H/del; mv $H/ren $H/ren2; chmod 755 $H/chm
container exec $N sh -c "
echo before: atom=\$(cat $G/atom 2>&1) del=\$(stat -c %i $G/del 2>&1) ren=\$(cat $G/ren 2>&1) ren2=\$(cat $G/ren2 2>&1) chm=\$(stat -c %a $G/chm)
$L $G >/dev/null
echo 'after $L': atom=\$(cat $G/atom 2>&1) del=\$(stat -c %i $G/del 2>&1) ren=\$(cat $G/ren 2>&1) ren2=\$(cat $G/ren2 2>&1) chm=\$(stat -c %a $G/chm)
"
