"""Linux host observation only; never reads argv, environment, sockets or payloads.
Start beside the diagnostic cargo command; SIGTERM ends observation.
"""
import json, os, platform, signal, time
from pathlib import Path
running = True
def stop(*_):
    global running
    running = False
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
def read(path):
    try:
        return Path(path).read_text()
    except (OSError, UnicodeError):
        return None
def emit(kind, **values):
    print(json.dumps(dict(kind=kind, wall_ms=time.time_ns()//1_000_000, **values)), flush=True)
models = sorted({line.split(':',1)[1].strip() for line in (read('/proc/cpuinfo') or '').splitlines() if line.startswith(('model name', 'Hardware'))})
emit('hardware', architecture=platform.machine(), cpu_models=models,
     kernel=platform.release(), self_cgroup=read('/proc/self/cgroup'),
     online_cpus=os.cpu_count(), affinity_cpus=sorted(os.sched_getaffinity(0)),
     cpu_max=read('/sys/fs/cgroup/cpu.max'), memory_max=read('/sys/fs/cgroup/memory.max'),
     cpuset=read('/sys/fs/cgroup/cpuset.cpus.effective'))
watched={'podman','buildah','conmon','crun','runc','cargo','rustc','musl-gcc','cc1','curl','bun','node','pinfold','codex','claude'}
while running:
    processes=[]
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        comm=(read(entry/'comm') or '').strip()
        if comm not in watched and not comm.startswith(('storage-chown', 'e2e-')):
            continue
        # stat contains comm but no argv; retain only PID, PPID, CPU ticks and start time.
        raw=read(entry/'stat')
        if raw is None:
            continue
        fields=raw.rsplit(')',1)[1].split()
        processes.append(dict(pid=int(entry.name), comm=comm, ppid=int(fields[1]),
                              user_ticks=int(fields[11]), system_ticks=int(fields[12]),
                              start_ticks=int(fields[19]), io=read(entry/'io'),
                              wchan=read(entry/'wchan')))
    net=[]
    for line in (read('/proc/net/dev') or '').splitlines()[2:]:
        name,counts=line.split(':',1)
        fields=counts.split()
        net.append(dict(interface=name.strip(), rx_bytes=int(fields[0]), tx_bytes=int(fields[8])))
    emit('sample', processes=processes, cpu_stat=read('/sys/fs/cgroup/cpu.stat'),
         host_cpu_stat=read('/proc/stat'), locks=read('/proc/locks'),
         pressure_cpu=read('/proc/pressure/cpu'), pressure_io=read('/proc/pressure/io'),
         pressure_memory=read('/proc/pressure/memory'), diskstats=read('/proc/diskstats'),
         network_counters=net, meminfo=read('/proc/meminfo'))
    time.sleep(1)
emit('complete')
