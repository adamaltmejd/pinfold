#!/bin/sh
# Fresh disposable CI only. Cargo runs as this nonroot user.
set -eu
out=${1:?usage: profile-suite.sh OUTPUT_DIRECTORY}
exec python3 - "$out" <<'PY'
import json, os, pathlib, re, select, shutil, signal, subprocess, sys, tempfile, time
out=pathlib.Path(sys.argv[1]).resolve()
out.mkdir(parents=True,exist_ok=True)
if os.getuid()==0 or os.environ.get('GITHUB_ACTIONS')!='true':
    raise SystemExit('Requires a nonroot disposable GitHub Actions job')
private=pathlib.Path(tempfile.mkdtemp(prefix='pf-perf-',dir=os.environ['RUNNER_TEMP']))
private.chmod(0o700)
perf=pathlib.Path(shutil.which('perf') or '/nonexistent-perf')
status={'profile':'unrun','suite_exit':None,'errors':[],
        'scope':'user CPU only; exact Pinfold comms; no callgraph, stack, argv or environment export'}
status['perf_command']=str(perf)
observer=profiler=suite=None
profiler_identity=None
handles=[]
ctl=ack=launch=None


def persist():
    (out/'profile-status.json').write_text(json.dumps(status,indent=2)+'\n')


def identity(pid):
    try:
        fields=pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
        return int(fields[19])
    except (OSError,ValueError,IndexError):
        return None


def stop_profiler():
    global profiler
    if profiler is None:return
    pid,start=profiler_identity or (None,None)
    if pid is not None and identity(pid)==start:
        stopped=subprocess.run(['sudo','-n','kill','-INT',str(pid)],stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL,timeout=5)
        if stopped.returncode:status['errors'].append('profiler INT failed')
    try:
        profiler.wait(timeout=15)
    except subprocess.TimeoutExpired:
        if pid is not None and identity(pid)==start:
            subprocess.run(['sudo','-n','kill','-TERM',str(pid)],stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL,timeout=5)
        try:profiler.wait(timeout=5)
        except subprocess.TimeoutExpired:
            if pid is not None and identity(pid)==start:
                subprocess.run(['sudo','-n','kill','-KILL',str(pid)],stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL,timeout=5)
            profiler.wait(timeout=5)
            status['errors'].append('profiler required forced shutdown')
    status['perf_record_exit']=profiler.returncode
    profiler=None


def stop_observer():
    global observer
    if observer is None:return
    if observer.poll() is None:
        observer.send_signal(signal.SIGTERM)
        try:observer.wait(timeout=5)
        except subprocess.TimeoutExpired:
            observer.kill();observer.wait(timeout=5)
            status['errors'].append('observer required forced shutdown')
    status['observer_exit']=observer.returncode
    observer=None


def interrupted(number,_):
    status['errors'].append(f'supervisor signal {number}')
    raise KeyboardInterrupt
signal.signal(signal.SIGTERM,interrupted)
signal.signal(signal.SIGINT,interrupted)
persist()
try:
    (out/'cpu-flags.txt').write_text('\n'.join(sorted({line.strip() for line in pathlib.Path('/proc/cpuinfo').read_text().splitlines()
        if line.startswith(('model name','Hardware','flags','Features'))}))+'\n')
    (out/'hardware.json').write_text(json.dumps({'uname':list(os.uname()),'cpus':os.cpu_count(),
        'affinity':sorted(os.sched_getaffinity(0))},indent=2)+'\n')
    observation=open(out/'host.jsonl','wb');handles.append(observation)
    observation_errors=open(out/'observer.stderr','wb');handles.append(observation_errors)
    observer=subprocess.Popen(['python3','scripts/diagnose-suite-host.py'],stdout=observation,
                              stderr=observation_errors)
    try:
        if not perf.is_file() or not os.access(perf,os.X_OK):
            status['errors'].append('matching distro perf unavailable; profile unrun')
        else:
            with open(private/'preflight.stderr','wb') as errors:
                preflight=subprocess.run(['sudo','-n',str(perf),'stat','-e','cpu-clock:u','--','true'],
                    stdout=subprocess.DEVNULL,stderr=errors,timeout=15)
            status['preflight_exit']=preflight.returncode
            if preflight.returncode:
                status['errors'].append('user-CPU perf preflight failed; profile unrun')
            else:
                for name in ('control','ack','launch'):os.mkfifo(private/name,0o600)
                ctl=os.open(private/'control',os.O_RDWR|os.O_NONBLOCK)
                ack=os.open(private/'ack',os.O_RDWR|os.O_NONBLOCK)
                launch=os.open(private/'launch',os.O_RDWR|os.O_NONBLOCK)
                errors=open(private/'record.stderr','wb');handles.append(errors)
                # The helper cannot exec perf until its exact identity is acknowledged.
                launcher=("import json,os,pathlib,select,sys\n"
                    "os.umask(0o077)\n"
                    "f=pathlib.Path('/proc/self/stat').read_text().rsplit(')',1)[1].split()\n"
                    "print(json.dumps([os.getpid(),int(f[19])]),flush=True)\n"
                    "fd=os.open(sys.argv[1],os.O_RDONLY|os.O_NONBLOCK)\n"
                    "ready=select.select([fd],[],[],15)[0]\n"
                    "go=os.read(fd,64) if ready else b''\n"
                    "os.close(fd)\n"
                    "if go!=b'go\\n':sys.exit(124)\n"
                    "os.dup2(os.open(os.devnull,os.O_WRONLY),1)\n"
                    "os.execv(sys.argv[2],sys.argv[2:])\n")
                profiler=subprocess.Popen(['sudo','-n','python3','-c',launcher,str(private/'launch'),str(perf),
                    'record','-a','-e','cpu-clock:u','-F','97','--delay=-1',
                    '--control=fifo:'+str(private/'control')+','+str(private/'ack'),
                    '-o',str(private/'perf.data')],stdout=subprocess.PIPE,stderr=errors)
                if select.select([profiler.stdout],[],[],15)[0]:
                    profiler_identity=json.loads(profiler.stdout.readline())
                if not profiler_identity:
                    raise RuntimeError('profiler identity unavailable')
                os.write(launch,b'go\n')
                os.write(ctl,b'enable\n')
                deadline=time.monotonic()+15
                ready=False
                while time.monotonic()<deadline:
                    if select.select([ack],[],[],min(0.2,max(0,deadline-time.monotonic())))[0]:
                        if b'ack' in os.read(ack,4096):ready=True;break
                    if profiler.poll() is not None:break
                if ready and profiler_identity:
                    status['profile']='recording'
                else:
                    status['errors'].append('perf record startup/control failed; profile unrun')
                    stop_profiler()
    except Exception as error:
        status['profile']='unrun'
        status['errors'].append('profiler startup error: '+type(error).__name__)
        try:stop_profiler()
        except Exception as cleanup:status['errors'].append('profiler startup cleanup error: '+type(cleanup).__name__)
    persist()
    if status['profile']!='recording':
        raise RuntimeError('profiling unavailable; do not run another unprofiled suite')
    with open(out/'suite.log','wb') as log:
        suite=subprocess.Popen(['/usr/bin/time','-f','elapsed=%e user=%U system=%S max_rss_kib=%M exit=%x',
            '-o',str(out/'command-time.txt'),'cargo','test','-p','e2e','--locked','--','--nocapture'],
            stdout=subprocess.PIPE,stderr=subprocess.STDOUT,start_new_session=True)
        for data in iter(lambda:suite.stdout.read(65536),b''):
            log.write(data);log.flush();sys.stdout.buffer.write(data);sys.stdout.buffer.flush()
        status['suite_exit']=suite.wait()
    stop_profiler()
    stop_observer()
    if status['profile']=='recording':
        with open(private/'report.stderr','wb') as errors:
            report=subprocess.run(['sudo','-n',str(perf),'report','--stdio','--no-children',
                '--sort','pid,comm,dso,symbol','--comms=pinfold,pinfold-init',
                '--field-separator',';','--percent-limit','0','-i',str(private/'perf.data')],
                stdout=subprocess.PIPE,stderr=errors,timeout=45)
        status['perf_report_exit']=report.returncode
        if report.returncode:
            status['profile']='report-failed';status['errors'].append('perf report failed')
        else:
            aggregate=[];unparsed=0
            for line in report.stdout.decode(errors='replace').splitlines():
                if not line.strip() or line.lstrip().startswith('#'):continue
                fields=[x.strip() for x in line.split(';')]
                while fields and not fields[-1]:fields.pop()
                if len(fields)!=5:unparsed+=1;continue
                overhead,pidfield,comm,dso,symbol=fields
                pidmatch=re.fullmatch(r'(?:[^:]*:)?\s*(\d+)',pidfield)
                if comm not in ('pinfold','pinfold-init') or not pidmatch:unparsed+=1;continue
                try:percentage=float(overhead.rstrip('%'))
                except ValueError:unparsed+=1;continue
                dso=pathlib.PurePosixPath(dso).name
                if '/' in symbol or '\\' in symbol:symbol='[path-bearing symbol omitted]'
                aggregate.append({'overhead_percent':percentage,'pid':int(pidmatch[1]),
                                  'comm':comm,'dso':dso,'symbol':symbol})
            (out/'perf-summary.json').write_text(json.dumps({'records':aggregate,
                'unparsed_lines':unparsed,'comm_filter':['pinfold','pinfold-init'],
                'notes':['PID attribution must be checked against sampled start_ticks; PID reuse can be ambiguous.',
                         'Aggregate user CPU samples; no wall-time or ownership/chown inference.']},indent=2)+'\n')
            if not aggregate or unparsed:
                status['profile']='report-format-error';status['errors'].append('perf report had empty/unparsed records')
            elif status.get('perf_record_exit')!=0:
                status['profile']='partial-recording';status['errors'].append('perf record did not exit successfully')
            else:status['profile']='complete'
except KeyboardInterrupt:
    if suite is not None and suite.poll() is None:
        os.killpg(suite.pid,signal.SIGTERM)
        try:status['suite_exit']=suite.wait(timeout=30)
        except subprocess.TimeoutExpired:
            os.killpg(suite.pid,signal.SIGKILL);status['suite_exit']=suite.wait(timeout=10)
    status['errors'].append('suite interrupted')
except Exception as error:
    if status['profile']=='recording':status['profile']='report-failed'
    status['errors'].append('supervision/report error: '+type(error).__name__)
finally:
    if suite is not None and suite.poll() is None:
        os.killpg(suite.pid,signal.SIGTERM)
        try:status['suite_exit']=suite.wait(timeout=30)
        except subprocess.TimeoutExpired:
            os.killpg(suite.pid,signal.SIGKILL);status['suite_exit']=suite.wait(timeout=10)
    for stop in (stop_profiler,stop_observer):
        try:stop()
        except Exception as error:status['errors'].append('cleanup failure: '+type(error).__name__)
    for fd in (ctl,ack,launch):
        if fd is not None:os.close(fd)
    for handle in handles:handle.close()
    try:shutil.rmtree(private);status['raw_profile_removed']=True
    except OSError:status['raw_profile_removed']=False;status['errors'].append('raw profile deletion failed')
    persist()
# Keep the suite result even when the diagnostic's private-data cleanup fails.
if not status.get('raw_profile_removed',False):raise SystemExit(126)
raise SystemExit(status['suite_exit'] if status['suite_exit'] is not None else 125)
PY
