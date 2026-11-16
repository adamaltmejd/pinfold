set -eu
W=/tmp/pfp322vm
mkdir -p "$W"
cd "$W"
gcc ${PROBE_CFLAGS:-} -static -O2 /tmp/pfp322-probe/controller.c -o controller
truncate -s 32M root.ext4
/usr/sbin/mkfs.ext4 -q -F root.ext4
cat > debugfs.commands <<'CMDS'
mkdir /dev
mkdir /proc
mkdir /sys
mkdir /sys/fs
mkdir /sys/fs/cgroup
mkdir /tmp
write controller /controller
set_inode_field /controller mode 0100755
CMDS
/usr/sbin/debugfs -w -f debugfs.commands root.ext4 >/dev/null 2>&1
python3 - <<'PY'
import json,pathlib
w=pathlib.Path('/tmp/pfp322vm');a=pathlib.Path.home()/'.cache/pinfold/artifacts/vmm/1.17.0+6.18.57/linux-x64'
c={'boot-source':{'kernel_image_path':str(a/'vmlinux'),'boot_args':'console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda ro init=/controller'},'drives':[{'drive_id':'root','path_on_host':str(w/'root.ext4'),'is_root_device':True,'is_read_only':True}],'machine-config':{'vcpu_count':2,'mem_size_mib':1024}}
(w/'config.json').write_text(json.dumps(c));(w/'runtime').write_text(str(a/'firecracker/firecracker-v1.17.0-x86_64'))
PY
setsid nohup "$(cat runtime)" --no-api --config-file config.json > controller.log 2>&1 < /dev/null &
echo $! > vmm.pid
