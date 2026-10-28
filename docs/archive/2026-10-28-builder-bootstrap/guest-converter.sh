#!/bin/bash
# No builder cache is attached. /input is the merged trial's golden OCI fixture.
set -x
mkdir -p /tmp/convert
cat >/tmp/convert/Dockerfile <<'EOF'
FROM --platform=linux/amd64 golden
EOF
manifest=sha256:871f87c0b192a668ac95bfe54d8e13b1ab1091cd3eade32182688fddb6e2769b
buildctl build --frontend dockerfile.v0 --local dockerfile=/tmp/convert --local context=/tmp/convert \
	--oci-layout image=/input --opt "context:golden=oci-layout://image@$manifest" \
	--output type=tar,dest=/work/root.tar
tar -tvf /work/root.tar
mkdir /work/tree /work/check
tar --numeric-owner --same-owner --same-permissions --xattrs --xattrs-include='*' \
	-xpf /work/root.tar -C /work/tree
truncate -s 128M /work/converted.ext4
mkfs.ext4 -q -F -d /work/tree /work/converted.ext4
mount -o ro,loop /work/converted.ext4 /work/check
ls -li /work/check
stat -c '%n:%u:%g:%a:%i' /work/check/owned /work/check/hard
test "$(stat -c '%u:%g:%a' /work/check/owned)" = 123:456:751
test "$(stat -c '%i' /work/check/owned)" = "$(stat -c '%i' /work/check/hard)"
test "$(readlink /work/check/sym)" = /owned
test ! -e /work/check/gone
test ! -e /work/check/dir/old
test "$(cat /work/check/dir/new)" = newer
test "$(debugfs -R 'ea_get /owned user.phase6' /work/converted.ext4 2>/dev/null)" = 'user.phase6 (3) = "yes"'
umount /work/check
kill "$daemon"
wait "$daemon" || true
wait "$logger"
set +x
printf '\nCONVERTED_BEGIN\n'
# Small writes let the virtual serial console yield between lines.
gzip -n -c /work/converted.ext4 | base64 | while IFS= read -r line; do
	printf '%s\n' "$line"
done
printf 'CONVERTED_END\n'
sync
umount /input /work
printf '\nCONVERTER_SUCCESS\n'
