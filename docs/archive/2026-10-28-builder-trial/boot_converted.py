#!/usr/bin/python3
"""Boot the received root, observe it, stop it; no BuildKit/cache/vsock/NIC."""
import hashlib,json,os,select,subprocess,time
from pathlib import Path
config={"boot-source":{"kernel_image_path":str(Path("vmlinux").resolve()),"boot_args":"console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda ro init=/sbin/probe-init"},"drives":[{"drive_id":"root","path_on_host":str(Path("outputs/convert-metadata.bin").resolve()),"is_root_device":True,"is_read_only":True}],"machine-config":{"vcpu_count":1,"mem_size_mib":256}}
Path("boot-converted.json").write_text(json.dumps(config))
t=time.monotonic();data=b""
p=subprocess.Popen(["./release-v1.17.0-x86_64/firecracker-v1.17.0-x86_64","--no-api","--config-file","boot-converted.json"],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
try:
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        if not select.select([p.stdout],[],[],max(0,deadline-time.monotonic()))[0]:break
        b=os.read(p.stdout.fileno(),65536)
        if not b:break
        data+=b
        if b"WHITEOUT_OK" in data:break
finally:
    p.terminate()
    try:p.wait(timeout=10)
    except subprocess.TimeoutExpired:p.kill();p.wait()
    data+=p.stdout.read()
Path("boot-converted-console.log").write_bytes(data)
text=data.decode(errors="replace")
print(json.dumps({"seconds":round(time.monotonic()-t,3),"returncode":p.returncode,"root_sha256":hashlib.file_digest(open("outputs/convert-metadata.bin","rb"),"sha256").hexdigest(),"marker":"CONVERTED_ROOT_BOOTED" in text,"whiteout":"WHITEOUT_OK" in text,"console_tail":"\n".join(text.splitlines()[-25:])}))
assert "CONVERTED_ROOT_BOOTED" in text and "WHITEOUT_OK" in text
