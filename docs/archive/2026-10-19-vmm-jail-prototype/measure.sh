#!/bin/bash
# Check 4: launch to guest-ready, jailed and unjailed, 10 runs each,
# alternating. The launcher's own setup time is in fcjail.err.
. ~/proto/lib.sh
for i in $(seq 1 10); do
  for j in jail --no-jail; do
    mkdata $ST/m.ext4 256M
    flag=; [ $j = --no-jail ] && flag=--no-jail
    r=$(up m A $ST/m.ext4 $flag)
    echo "$j run=$i $(echo $r | cut -d' ' -f1) $(grep -o 'setup_us=[0-9]*' $ST/m/fcjail.err) vmm_rss_kib=$(awk '/VmRSS/{print $2}' /proc/$(cat $ST/m/vmm.pid)/status)"
    kill_vm m
  done
done | tee $ST/measure.raw
for j in jail --no-jail; do
  grep "^$j " $ST/measure.raw | sed 's/.*ready_ms=\([0-9]*\).*setup_us=\([0-9]*\).*/\1 \2/' | sort -n |
    awk -v j=$j '{r[NR]=$1; s+=$2; t+=$1} END {printf "%s ready_ms min=%d median=%d max=%d mean=%.0f; launcher setup mean_us=%.0f\n", j, r[1], (r[5]+r[6])/2, r[NR], t/NR, s/NR}'
done
