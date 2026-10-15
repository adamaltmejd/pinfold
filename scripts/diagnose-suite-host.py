"""Linux host observation only; reads Podman argv only for fixed operation classification.
Never emits argv or reads environment, sockets or payloads.
Start beside the diagnostic cargo command; SIGTERM ends observation.
"""
import json, os, platform, signal, time
from pathlib import Path, PurePosixPath
import re
running = True
def stop(*_):
    global running
    running = False
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
def read(path, limit=1024 * 1024):
    try:
        with Path(path).open('rb') as stream:
            data = stream.read(limit + 1)
        return data.decode() if len(data) <= limit else None
    except (OSError, UnicodeError):
        return None


def podman_operation(entry):
    # Discard all bytes after exact positional classification. Never log errors.
    try:
        with (entry / 'cmdline').open('rb') as stream:
            data = stream.read(65537)
    except OSError:
        return 'unknown'
    if len(data) > 65536 or not data.endswith(b'\0'):
        return 'unknown'
    argv = data.split(b'\0')
    verbs = {b'info': 'podman.info', b'build': 'podman.build',
             b'run': 'podman.run', b'exec': 'podman.exec', b'ps': 'podman.ps',
             b'inspect': 'podman.inspect', b'rm': 'podman.rm'}
    image_verbs = {b'inspect': 'podman.image.inspect', b'list': 'podman.image.list',
                   b'rm': 'podman.image.rm', b'prune': 'podman.image.prune'}
    if len(argv) > 2 and argv[1] == b'image':
        return image_verbs.get(argv[2], 'unknown')
    return verbs.get(argv[1], 'unknown') if len(argv) > 1 else 'unknown'


def mount_unescape(value):
    return re.sub(r'\\([0-7]{3})', lambda match: chr(int(match[1], 8)), value)


def cgroup_directory():
    membership = read('/proc/self/cgroup')
    mounts = read('/proc/self/mountinfo')
    if membership is None or mounts is None:
        return None
    paths = [line[3:] for line in membership.splitlines() if line.startswith('0::')]
    if len(paths) != 1:
        return None
    member = PurePosixPath(paths[0])
    if not member.is_absolute() or '..' in member.parts:
        return None
    candidates = []
    for line in mounts.splitlines():
        before, separator, after = line.partition(' - ')
        left, right = before.split(), after.split()
        if not separator or len(left) < 5 or not right or right[0] != 'cgroup2':
            continue
        root = PurePosixPath(mount_unescape(left[3]))
        mount = Path(mount_unescape(left[4]))
        if not root.is_absolute() or '..' in root.parts or not mount.is_absolute():
            continue
        try:
            relative = member.relative_to(root)
            mount = mount.resolve()
            current = mount.joinpath(*relative.parts).resolve()
            current.relative_to(mount)
        except (ValueError, OSError, RuntimeError):
            continue
        candidates.append((len(root.parts), mount, current))
    if not candidates:
        return None
    _, mount, current = max(candidates, key=lambda row: row[0])
    return mount, current


def visible_cpu_limits(mapping):
    # A visible ceiling says nothing about limits hidden above this namespace.
    if mapping is None:
        return {'status': 'unavailable', 'outside_visible_mount': 'not visible'}
    mount, current = mapping
    levels, ceilings = [], []
    unavailable = False
    depth = 0
    while True:
        raw = read(current / 'cpu.max', 4096)
        fields = raw.split() if raw is not None else []
        limit = {'status': 'unavailable'}
        if len(fields) == 2:
            try:
                period = int(fields[1])
                if period > 0 and fields[0] == 'max':
                    limit = {'status': 'max', 'period_us': period}
                elif period > 0 and int(fields[0]) > 0:
                    quota = int(fields[0])
                    limit = {'status': 'finite', 'quota_us': quota, 'period_us': period}
                    ceilings.append(quota / period)
            except ValueError:
                pass
        unavailable |= limit['status'] == 'unavailable'
        levels.append({'depth_from_self': depth, 'path': str(current),
                       'cpu_max': limit, 'cpu_stat': read(current / 'cpu.stat', 65536)})
        if current == mount:
            break
        current = current.parent
        depth += 1
    return {'status': 'partial' if unavailable else 'visible', 'levels': levels,
            'visible_finite_ceiling_cpus': min(ceilings) if ceilings else None,
            'outside_visible_mount': 'not visible'}


cgroup_mapping = cgroup_directory()
def emit(kind, **values):
    print(json.dumps(dict(kind=kind, wall_ms=time.time_ns()//1_000_000, monotonic_ns=time.monotonic_ns(), **values)), flush=True)
models = sorted({line.split(':',1)[1].strip() for line in (read('/proc/cpuinfo') or '').splitlines() if line.startswith(('model name', 'Hardware'))})
emit('hardware', architecture=platform.machine(), cpu_models=models,
     kernel=platform.release(), self_cgroup=read('/proc/self/cgroup'),
     online_cpus=os.cpu_count(), affinity_cpus=sorted(os.sched_getaffinity(0)),
     clock_ticks_per_second=os.sysconf('SC_CLK_TCK'),
     cpu_max_scope='legacy mount-root observation; not effective quota',
     cgroup_cpu=visible_cpu_limits(cgroup_mapping),
     cpu_max=read('/sys/fs/cgroup/cpu.max'), memory_max=read('/sys/fs/cgroup/memory.max'),
     cpuset=read('/sys/fs/cgroup/cpuset.cpus.effective'))
watched={'podman','buildah','conmon','crun','runc','cargo','rustc','musl-gcc','cc1','curl','bun','node','pi','apt-get','apt','dpkg','pinfold','codex','claude','fuse-overlayfs','pinfold-init'}
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
        try:
            fields=raw.rsplit(')',1)[1].split()
            process = dict(pid=int(entry.name), comm=comm, ppid=int(fields[1]), state=fields[0],
                           user_ticks=int(fields[11]), system_ticks=int(fields[12]),
                           start_ticks=int(fields[19]), io=read(entry/'io'),
                           wchan=read(entry/'wchan'))
        except (ValueError, IndexError):
            continue
        if comm == 'podman':
            process['operation'] = podman_operation(entry)
        if comm in {'pinfold', 'pinfold-init'}:
            for field in ('uid_map', 'gid_map'):
                mapping = read(entry / field, 4096)
                if mapping is not None and all(
                    len(parts := line.split()) == 3
                    and all(value.isascii() and value.isdecimal() for value in parts)
                    for line in mapping.splitlines()
                ):
                    process[field] = mapping
                else:
                    process[field] = None
        processes.append(process)
    net=[]
    for line in (read('/proc/net/dev') or '').splitlines()[2:]:
        name,counts=line.split(':',1)
        fields=counts.split()
        net.append(dict(interface=name.strip(), rx_bytes=int(fields[0]), tx_bytes=int(fields[8])))
    emit('sample', processes=processes, cgroup_cpu=visible_cpu_limits(cgroup_mapping),
         cpu_stat=read('/sys/fs/cgroup/cpu.stat'),
         host_cpu_stat=read('/proc/stat'), locks=read('/proc/locks'),
         pressure_cpu=read('/proc/pressure/cpu'), pressure_io=read('/proc/pressure/io'),
         pressure_memory=read('/proc/pressure/memory'), diskstats=read('/proc/diskstats'),
         network_counters=net, meminfo=read('/proc/meminfo'))
    time.sleep(1)
emit('complete')
