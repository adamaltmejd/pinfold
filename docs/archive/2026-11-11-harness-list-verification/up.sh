#!/bin/bash
# usage: up.sh SPEC OUT
# Starts `box up` with the spec on stdin and stdin held open by a sleeper,
# waits for the first stdout line, prints it and the elapsed seconds.
spec=$1 out=$2
start=$(python3 -c 'import time;print(time.time())')
( cat "$spec"; exec sleep 3617 ) | "$P" box up > "$out" 2> "$out.err" &
while ! [ -s "$out" ]; do sleep 0.05; done
end=$(python3 -c 'import time;print(time.time())')
head -1 "$out"
python3 -c "print('elapsed %.2f s' % ($end-$start))"
