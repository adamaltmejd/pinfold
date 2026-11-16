#!/usr/bin/env python3
"""Public transfer controls for the shared helper lifetime: BIN IMAGE."""
import json,os,pathlib,subprocess as sp,sys
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0],str(old),'exec'),globals())
for p in (P,INTO):p.mkdir();git(p,'init','-q','-b','review')
(P/'tracked').write_bytes(b'FIXTURE\n');git(P,'add','tracked');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','-qm','fixture')
run(['config','edit']);c=W/'c/pinfold/projects'/ident(P)/'pinfold.toml';c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
try:
 run(['start']);observed=loc(P)
 for verb in ('push','fetch'):
  result=json.loads(run([verb]).stdout);assert result['event']==('pushed' if verb=='push' else 'fetched') and result['box']==observed['name'];same(P,observed)
 guest=run(['--','git','rev-parse','HEAD']).stdout.strip();branch=run(['--','git','symbolic-ref','--short','HEAD']).stdout.strip().decode()
 result=json.loads(run(['box','fetch',observed['name'],'--repo',str(P),'--into',str(INTO)]).stdout)
 ref=f'refs/remotes/{observed["name"]}/{branch}'
 assert result['event']=='fetched' and result['refs'][ref].encode()==guest and git(INTO,'rev-parse',ref)==guest;same(P,observed)
 emit('transfer controls: project push=0/pushed fetch=0/fetched; public box fetch=0/fetched; guest HEAD=independent host remote ref; generation unchanged')
finally:
 run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 emit('cleanup exact own boxes/disks=True')
