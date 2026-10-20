#!/bin/bash
# Boot check for pinfold's guest kernel under Firecracker v1.17.0.
set -eu
cd ~/fc
rm -rf root && mkdir -p root/bin root/dev root/proc root/sys root/lib/socat
cp /usr/bin/busybox root/bin/busybox
cp /usr/bin/socat1 root/bin/socat 2>/dev/null || cp "$(readlink -f /usr/bin/socat)" root/bin/socat
for l in $(ldd /usr/bin/socat | grep -o '/[^ ]*'); do cp "$l" root/lib/socat/; done
install -m755 init root/init
rm -f root.ext4 && /usr/sbin/mkfs.ext4 -q -d root root.ext4 32M
cat >vm.json <<J
{"boot-source":{"kernel_image_path":"$PWD/vmlinux","boot_args":"console=ttyS0 reboot=k panic=1 init=/init"},
 "drives":[{"drive_id":"root","path_on_host":"$PWD/root.ext4","is_root_device":true,"is_read_only":true}],
 "machine-config":{"vcpu_count":1,"mem_size_mib":256},
 "vsock":{"guest_cid":3,"uds_path":"$PWD/v.sock"}}
J
rm -f v.sock v.sock_3128
socat -u UNIX-LISTEN:v.sock_3128 OPEN:host-received.txt,creat,trunc &
timeout 60 ./firecracker --no-api --config-file vm.json
wait || true
echo "host received: $(cat host-received.txt)"
