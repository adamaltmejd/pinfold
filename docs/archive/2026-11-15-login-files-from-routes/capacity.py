#!/usr/bin/env python3
"""Manual observation of the pinned VMM limit; 14 is not a replacement for 24."""
import json, pathlib, select, subprocess, tempfile
import accept as a
fixture=a.W/'empty'; fixture.mkdir()
a.spec['egress']['routes'].update({
    'pi':{'login':'pi','provider':'anthropic','from':'FIXTURE_HOST_TOKEN','to':'http://127.0.0.1:9'},
    'claude':{'login':'claude','from':'FIXTURE_HOST_TOKEN','to':'http://127.0.0.1:9'}})
for total in (14,15,24,25):
    a.spec['mounts']=[{'host':str(fixture),'guest':'/fixture-'+str(i)} for i in range(total-3)]
    err=tempfile.TemporaryFile(mode='w+t')
    p=subprocess.Popen([str(a.P),'box','up'],env=a.E,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,text=True)
    try:
        p.stdin.write(json.dumps(a.spec)+'\n'); p.stdin.flush()
        a.check(select.select([p.stdout],[],[],180)[0],'capacity readiness deadline')
        row=json.loads(p.stdout.readline())
        a.check(row['event']==('ready' if total==14 else 'refused' if total==25 else 'failed'),'observed MMIO boundary')
        if total==25: a.check(row.get('reason')=='spec','25 attachment refusal reason')
        if total==14:
            a.check(a.x('printf allowed').stdout=='allowed','14 attachment exec control')
        a.emit('attachments='+str(total)+' event='+row['event']+' reason='+str(row.get('reason'))+' detail='+row.get('detail','exec-control-passed'))
    finally:
        a.run(['box','down',a.NAME],required=False)
        p.stdin.close(); p.wait(timeout=90); err.close()
        a.run(['disk','rm',a.DISK],required=False)
a.emit('capacity owned boxes/disks removed; attachment-24 acceptance remains failed')
a.spec['mounts']=[]; a.spec['egress']['routes'].clear()
a.spec['env']['HOME']='/state'; a.spec['disks'][0]['guest']='/state'
try:
    a.up(); a.x('mkdir -p /state/home'); a.down()
    a.spec['disks'][0]['readonly']=True
    a.spec['disks'].append({'name':a.DISK+'-child','guest':'/state/home'})
    a.spec['env']['HOME']='/state/home'
    a.spec['egress']['routes']['pi']={'login':'pi','provider':'anthropic','from':'FIXTURE_HOST_TOKEN','to':'http://127.0.0.1:9'}
    a.up(); a.x('test -r "$HOME/.pi/agent/extensions/pinfold-login/index.ts"; printf writable > "$HOME/ok"')
    a.emit('readonly ancestor with deeper writable HOME backing passed')
finally:
    a.down()
    for disk in (a.DISK,a.DISK+'-child'): a.run(['disk','rm',disk],required=False)
