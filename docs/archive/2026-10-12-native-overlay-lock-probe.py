import json
import os
from pathlib import Path
import signal
import subprocess
import tarfile
import time

os.environ.update(XDG_RUNTIME_DIR='/run/user/1000', DBUS_SESSION_BUS_ADDRESS='unix:path=/run/user/1000/bus')

def run(args, timeout=180):
    started = time.monotonic()
    result = subprocess.run(args, text=True, capture_output=True, timeout=timeout)
    print(json.dumps({'command':args, 'exit':result.returncode, 'seconds':round(time.monotonic()-started,3), 'stdout':result.stdout.strip(), 'stderr':result.stderr.strip()}), flush=True)
    result.check_returncode()
    return result.stdout.strip()

info = json.loads(subprocess.check_output(['podman','info','--format','json']))
store = Path(info['store']['graphRoot'])
driver = info['store']['graphDriverName']
print(json.dumps({'version':info['version']['Version'], 'driver':driver, 'status':info['store']['graphStatus'], 'rootless':info['host']['security']['rootless']}), flush=True)
archive = Path('/tmp/native-mapping.tar')
count = int(os.environ.get('FIXTURE_FILES','250000'))
with tarfile.open(archive, 'w', format=tarfile.USTAR_FORMAT) as tar:
    for d in range(count//1000):
        entry = tarfile.TarInfo('files/d'+str(d))
        entry.type = tarfile.DIRTYPE
        entry.mode = 0o755
        tar.addfile(entry)
        for f in range(1000):
            entry = tarfile.TarInfo('files/d'+str(d)+'/f'+str(f))
            entry.mode = 0o644
            tar.addfile(entry)
print(json.dumps({'fixture_files':count, 'tar_bytes':archive.stat().st_size, 'file_owner':0}), flush=True)
image = run(['podman','import',str(archive),'localhost/native-mapping:probe'])

identities = {}
for path in store.glob(driver+'-*/*.lock'):
    stat = path.stat()
    identities[f'{os.major(stat.st_dev):02x}:{os.minor(stat.st_dev):02x}:{stat.st_ino}'] = str(path.relative_to(store))

def snapshot():
    locks = []
    for line in Path('/proc/locks').read_text().splitlines():
        fields = line.split()
        identity = next((field for field in fields if field in identities), None)
        if identity:
            offset = 1 if '->' in fields else 0
            pid = fields[4+offset]
            proc = Path('/proc')/pid
            try:
                comm = (proc/'comm').read_text().strip()
                wchan = (proc/'wchan').read_text().strip()
            except OSError:
                comm, wchan = 'gone', 'gone'
            locks.append({'lock':identities[identity], 'identity':identity, 'waiter':'->' in fields, 'mode':fields[3+offset], 'pid':int(pid), 'comm':comm, 'wchan':wchan})
    return locks

def mapping_helpers():
    helpers = []
    for proc in Path('/proc').glob('[0-9]*'):
        try:
            role = (proc/'cmdline').read_bytes().split(b'\0')[0]
            if role != b'storage-chown-by-maps':
                continue
            stat = (proc/'stat').read_text().rsplit(')',1)[1].split()
            threads = []
            for task in (proc/'task').glob('[0-9]*'):
                try:
                    threads.append({'tid':int(task.name),'wchan':(task/'wchan').read_text().strip(),'syscall':(task/'syscall').read_text().split()[0]})
                except OSError:
                    pass
            helpers.append({'pid':int(proc.name),'ppid':int(stat[1]),'start_ticks':stat[19],'role':'storage-chown-by-maps','threads':threads})
        except (OSError,IndexError):
            pass
    return helpers

for phase in ['cold','warm']:
    args = ['podman','create','--name','native-mapping-'+phase,'--userns=keep-id','--network=none','--entrypoint=/unused',image]
    start = time.monotonic()
    with open('/tmp/create-'+phase+'.out','w+') as out, open('/tmp/create-'+phase+'.err','w+') as err:
        creator = subprocess.Popen(args, stdout=out, stderr=err, start_new_session=True)
        listing = None
        list_start = None
        create_finish = None
        list_finish = None
        hold_start = None
        candidate = None
        candidate_start = None
        candidate_finish = None
        events = []
        previous = None
        last_sample = 0
        try:
            while creator.poll() is None or (listing is not None and listing.poll() is None) or (candidate is not None and candidate.poll() is None):
                now = time.monotonic()
                if now-start > 180:
                    raise TimeoutError('workload observer reached 180 seconds')
                locks = snapshot()
                helpers = mapping_helpers()
                key = json.dumps({'locks':locks,'helpers':[h['pid'] for h in helpers]}, sort_keys=True)
                if key != previous or now-last_sample >= 5:
                    events.append({'seconds':round(now-start,3), 'locks':locks, 'helpers':helpers})
                    previous = key
                    last_sample = now
                creator_holds = any(l['lock'].endswith('images/images.lock') and not l['waiter'] and l['mode']=='WRITE' and l['pid']==creator.pid for l in locks)
                if creator_holds and hold_start is None:
                    hold_start = now
                elif not creator_holds:
                    hold_start = None
                if listing is None and hold_start is not None and now-hold_start >= 0.2:
                    list_start = time.monotonic()
                    listing = subprocess.Popen(['podman','ps','--all','--format','json'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
                    if Path('/tmp/pinfold-candidate').exists():
                        candidate_start = time.monotonic()
                        candidate = subprocess.Popen(['/tmp/pinfold-candidate','box','list','--label','dev.pinfold.owner'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True, env={**os.environ,'XDG_STATE_HOME':'/tmp/pf-mapping/state','XDG_CACHE_HOME':'/tmp/pf-mapping/cache','XDG_CONFIG_HOME':'/tmp/pf-mapping/config'})
                if creator.poll() is not None and create_finish is None:
                    create_finish = now
                    events.append({'create_exit':creator.returncode, 'seconds':round(now-start,3)})
                if listing is not None and listing.poll() is not None and list_finish is None:
                    list_finish = now
                if candidate is not None and candidate.poll() is not None and candidate_finish is None:
                    candidate_finish = now
                time.sleep(0.02)
            if listing is None:
                list_start = time.monotonic()
                listing = subprocess.Popen(['podman','ps','--all','--format','json'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
                listing.wait(timeout=30)
                list_finish = time.monotonic()
            create_finish = create_finish or time.monotonic()
            list_finish = list_finish or time.monotonic()
            out.seek(0)
            err.seek(0)
            print(json.dumps({'phase':phase,'create_pid':creator.pid,'create_exit':creator.returncode,'create_seconds':round(create_finish-start,3),'list_pid':listing.pid,'list_exit':listing.returncode,'list_seconds':round(list_finish-list_start,3),'stdout':out.read().strip(),'stderr':err.read().strip(),'events':events}), flush=True)
            images = json.loads((store/(driver+'-images')/'images.json').read_text())
            print(json.dumps({'phase':phase,'mapped_layers':[i.get('mapped-layers',[]) for i in images]}), flush=True)
            if candidate is not None:
                candidate_finish = candidate_finish or time.monotonic()
                candidate_out,candidate_err = candidate.communicate()
                print(json.dumps({'phase':phase,'candidate_pid':candidate.pid,'candidate_exit':candidate.returncode,'candidate_seconds':round(candidate_finish-candidate_start,3),'stdout':candidate_out,'stderr':candidate_err}),flush=True)
        finally:
            for child in [creator,listing,candidate]:
                if child is not None and child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
run(['podman','rm','native-mapping-cold','native-mapping-warm'])
