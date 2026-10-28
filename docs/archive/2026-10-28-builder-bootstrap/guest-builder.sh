#!/bin/bash
# Sourced after guest-common.sh in the disposable proof.
mount /dev/vdd /var/lib/buildkit
buildctl build --frontend dockerfile.v0 --local context=/input \
	--local dockerfile=/input --output type=oci,dest=/var/lib/buildkit/oci,tar=false
test -f /var/lib/buildkit/oci/index.json
kill "$daemon"
wait "$daemon" || true
wait "$logger"
sync
umount /var/lib/buildkit /input /work
printf '\nBUILDER_SUCCESS\n'
