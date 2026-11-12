#!/bin/bash
# usage: up.sh SPEC OUT [PREFIX…]
# Starts `box up` (behind PREFIX, e.g. strace) with the spec on stdin and
# stdin held open by a sleeper. Blocks on the first stdout line, prints it
# and the seconds from spawn to that line. The rest of stdout goes to OUT.
spec=$1 out=$2; shift 2
fifo=$(mktemp -u) && mkfifo "$fifo"
start=$EPOCHREALTIME
( cat "$spec"; exec sleep 3617 ) | "$@" "$P" box up > "$fifo" 2> "$out.err" &
exec 3< "$fifo"
IFS= read -r -u 3 line
end=$EPOCHREALTIME
rm "$fifo"
echo "$line" > "$out"
( cat <&3 >> "$out" ) &
exec 3<&-
echo "$line"
us=$(( ${end/./} - ${start/./} ))
printf 'elapsed %d.%03d s\n' $((us / 1000000)) $((us % 1000000 / 1000))
