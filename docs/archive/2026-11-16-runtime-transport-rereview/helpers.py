#!/usr/bin/env python3
"""Manual real-binary GDB cases. Run: python3 helpers.py fork|builder BIN IMAGE [ROOT]."""
from pathlib import Path
import sys
mode=sys.argv.pop(1)
prior=Path(__file__).resolve().parent.parent/'2026-11-16-runtime-transport-review/crash-windows.py'
# Import the earlier driver's setup/functions without executing its cases.
# Use this module's globals so image can be changed for the built-image control.
exec(compile(prior.read_text().split('# Anchors intentionally')[0],str(prior),'exec'),globals())

if mode=='fork':
    name=prefix+'-fork-refusal';spec=work/'fork.json';spec.write_text(json.dumps(plan(name))+'\n')
    out,err=work/'fork.out',work/'fork.err'
    line=anchor('let pid = syscall(unsafe { libc::fork() })?;')
    commands=work/'fork.gdb'
    commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
break {source}:{line}
run box up < {shlex.quote(str(spec))} > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
set scheduler-locking on
catch syscall clone fork clone3
continue
set $orig_rax = -1
delete breakpoints
set scheduler-locking off
continue
python
import gdb
assert int(gdb.parse_and_eval('$_exitcode'))==1
end
''')
    r=sp.run(['gdb','-nx','-q','-batch','-x',str(commands),'--args',binary],env=env,cwd=repo,capture_output=True,timeout=120)
    emit('$ gdb syscall-entry barrier: suppress only bind helper fork; kernel returns ENOSYS')
    emit(f'gdb exit={r.returncode} owner stdout={out.read_bytes()!r} stderr={err.read_bytes()!r}')
    assert r.returncode==0 and b'runtime-path' not in out.read_bytes()+err.read_bytes(),r.stdout+r.stderr
    assert json.loads(out.read_bytes())['event']=='failed' and b'Function not implemented' in out.read_bytes()
    corrected_control(name)
else:
    assert mode=='builder'
    name=f'probe-rereview-boot-{os.getpid()}';recipe=work/'Containerfile'
    recipe.write_text(f'FROM {image}\nRUN printf BOOT-BUILD-CONTROL >/probe-boot-marker\n')
    job=repo/'crates/pinfold/src/core/image/linux/job.rs'
    before=anchor('let listener = crate::core::runtime::sockets::bind_at(',job)
    after=anchor('let mut drives = vec![(work.to_path_buf(), true), (disk, false)];',job)
    target=work/'boot-target';target.mkdir(mode=0o700);(target/'sentinel').write_bytes(b'FOREIGN-TARGET')
    observation=work/'builder-observation.json';commands=work/'builder.gdb'
    out,err=work/'builder.out',work/'builder.err'
    args=['image','build',name,'--containerfile',str(recipe),'--context',str(work)]
    commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,os,pathlib,stat
os.umask(0o022)
state=None
class Before(gdb.Breakpoint):
 def stop(self):
  global state
  if state is not None:return False
  if 'Build' not in str(gdb.parse_and_eval('phase')):return False
  base=pathlib.Path({str(host)!r})
  records=[json.loads(p.read_text()) for p in base.glob('*.owner')]
  records=[r for r in records if r['name'].startswith('builder-')]
  assert len(records)==1,records
  dirs=list(base.glob('b-*-'+records[0]['generation'][:24]+'-b'))
  if not dirs:return False
  original=dirs[0];moved=base/('probe-retained-'+records[0]['generation'])
  original.rename(moved);original.symlink_to({str(target)!r},target_is_directory=True)
  state={{'original':str(original),'moved':str(moved)}}
  pathlib.Path({str(observation)!r}).write_text(json.dumps(state))
  return False
class After(gdb.Breakpoint):
 def stop(self):
  if state is None or state.get('restored'):return False
  original=pathlib.Path(state['original']);moved=pathlib.Path(state['moved'])
  socket=moved/'v.sock_1000';assert stat.S_ISSOCK(socket.lstat().st_mode)
  assert stat.S_IMODE(socket.stat().st_mode)==0o600
  target=pathlib.Path({str(target)!r});assert sorted(p.name for p in target.iterdir())==['sentinel']
  assert (target/'sentinel').read_bytes()==b'FOREIGN-TARGET'
  original.unlink();moved.rename(original)
  state.update(restored=True,boot_mode='0600',retained_binding=True,target_unchanged=True)
  pathlib.Path({str(observation)!r}).write_text(json.dumps(state))
  gdb.write('EXTERNAL-BARRIER BOOT0600 retained-directory target-unchanged\\n')
  return False
Before({str(job)+':'+str(before)!r});After({str(job)+':'+str(after)!r})
end
run {shlex.join(args)} > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
python
assert int(gdb.parse_and_eval('$_exitcode'))==0
end
''')
    try:
        r=sp.run(['gdb','-nx','-q','-batch','-x',str(commands),'--args',binary]+args,env=env,cwd=repo,capture_output=True,timeout=600)
    finally:
        if observation.exists():
            observed=json.loads(observation.read_text());original=Path(observed['original']);moved=Path(observed['moved'])
            if original.is_symlink() and moved.exists():original.unlink();moved.rename(original)
    emit('$ gdb image build: source barriers replace only our retained socket-directory pathname, then restore before VMM spawn')
    emit(f'gdb exit={r.returncode} build stdout={out.read_bytes()!r}')
    assert r.returncode==0,r.stdout+r.stderr+err.read_bytes()
    observed=json.loads(observation.read_text());assert observed.get('restored') and observed.get('retained_binding')
    emit('5 BOOT socket0600 under umask022; bind used retained directory; symlink target unchanged; real RUN build succeeded')
    image=f'localhost/pinfold/image-{name}:latest';boxname=prefix+'-built-control'
    child=start(boxname)
    try:assert run(['box','exec',boxname,'--','cat','/probe-boot-marker']).stdout==b'BOOT-BUILD-CONTROL'
    finally:finish(boxname,child)
    emit('5 built-image execution control passed; retained own image '+image)
