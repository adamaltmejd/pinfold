#!/bin/bash
# Q5: within the stale window after a host atomic replace, compare lookup vs readdir. Usage: diag.sh CONTAINER HOSTDIR TAG
N=$1; H=$2/$3; G=/work/$3; mkdir -p $H; echo old > $H/atom; echo keep > $H/other; echo d > $H/del
echo "host before: $(stat -f 'atom ino=%i size=%z' $H/atom)"
echo "guest prime: $(container exec $N sh -c "stat -c 'atom ino=%i size=%s' $G/atom; cat $G/atom; stat -c 'del ino=%i' $G/del" | tr '\n' ' ')"
echo newer-and-longer > $H/atom.tmp; mv -f $H/atom.tmp $H/atom; rm $H/del
echo "host after: $(stat -f 'atom ino=%i size=%z' $H/atom); del exists=$([ -e $H/del ] && echo y || echo n)"
container exec $N sh -c "
echo '-- stat atom:'; stat -c 'ino=%i size=%s' $G/atom 2>&1
echo '-- cat atom:'; cat $G/atom 2>&1
echo '-- cat via ./ and // paths:'; cat $G/./atom 2>&1; cat /work//$3/atom 2>&1
echo '-- readdir (ls -1fi, no stat per entry? ls -i uses d_ino):'; ls -1fi $G 2>&1
echo '-- ls -li (readdir + lstat each):'; ls -li $G 2>&1
echo '-- find -name atom -printf inode (readdir d_ino):'; find $G -maxdepth 1 -name atom -printf '%i %s\n' 2>&1
echo '-- stat del:'; stat -c 'ino=%i' $G/del 2>&1
echo '-- cat del:'; cat $G/del 2>&1
echo '-- stat dir:'; stat -c 'dir ino=%i nlink=%h mtime=%Y size=%s' $G 2>&1
"
echo "host dir: $(stat -f 'dir ino=%i nlink=%l mtime=%m size=%z' $H)"
sleep 1.5
echo "== after 1.5 s"; container exec $N sh -c "stat -c 'atom ino=%i size=%s' $G/atom 2>&1; cat $G/atom; stat $G/del 2>&1 | head -1"
