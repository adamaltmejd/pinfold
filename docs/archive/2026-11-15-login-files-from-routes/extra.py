#!/usr/bin/env python3
"""Manual TTY/entrypoint and real host-helper cancellation acceptance."""
import datetime, fcntl, json, os, pathlib, pty, select, signal, struct, subprocess
import sys, termios, threading, time, re
import accept as a
def tty(args, cwd=None, expected=(), attached=False):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH',36,120,0,0))
    def session():
        os.setsid(); fcntl.ioctl(0,termios.TIOCSCTTY,0); os.tcsetpgrp(0,os.getpgrp())
    p = subprocess.Popen([str(a.P),*args],env={**a.E,'TERM':'xterm-256color'},cwd=cwd or a.W,
        stdin=slave,stdout=slave,stderr=slave,preexec_fn=session)
    os.close(slave); output=b''; end=time.monotonic()+40; matched=False
    try:
        while time.monotonic()<end and p.poll() is None:
            if not select.select([master],[],[],1)[0]: continue
            try: chunk=os.read(master,65536)
            except OSError: break
            output+=chunk
            for query,reply in ((b'\x1b[6n',b'\x1b[1;1R'),(b'\x1b[c',b'\x1b[?1;2c'),(b'\x1b[?u',b'\x1b[?0u'),
                (b'\x1b]11;?',b'\x1b]11;rgb:0000/0000/0000\x1b\\')):
                if query in chunk: os.write(master,reply)
            screen=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07)|\x1b.', ' ', output.decode(errors='replace'))
            if not matched and any(''.join(v.split()) in ''.join(screen.split()) for v in expected):
                matched=True
                if attached:
                    q=subprocess.run([str(a.P),'attach','sh','-c','printf attached; pi --version; env'],env=a.E,cwd=cwd,capture_output=True,text=True,timeout=60)
                    a.check(q.returncode==0 and 'attached' in q.stdout and 'PINFOLD_ALLOW=' in q.stdout,'attach path/env')
                    a.check(not any(m in q.stdout.encode() for m in a.MARKERS),'attach marker scan')
                    a.emit('pinfold attach command PATH and environment passed')
                if attached: break
        if not matched: a.emit(output.decode(errors='replace')[-2000:])
        a.check(matched,'interactive startup expected screen')
        a.emit('interactive '+args[-1]+' recognized=true bytes='+str(len(output)))
    finally:
        os.close(master)
        if p.poll() is None: p.send_signal(signal.SIGTERM)
        try: p.wait(timeout=30)
        except subprocess.TimeoutExpired: p.kill(); p.wait(timeout=10)
for label,agent,provider in [('claude-login','claude',None),('codex-login','codex',None),('pi-login','pi','anthropic')]:
    a.fixtures[label], a.routes[label]=a.route(label,agent,provider)
a.spec['profile']='default'
project_disks=[]
try:
    a.up()
    pins=json.loads(a.run(['artifacts']).stdout)
    helper=next(p for p in pins if p['name']=='codex')
    arch='host-linux-x64' if sys.platform.startswith('linux') else 'host-darwin-arm64'
    a.check(arch in helper['path'] and helper['path'].endswith('/codex-app-server') and helper['cached'],'host helper architecture/path')
    a.check(not any(p['name'] in ('pi','claude') for p in pins),'guest pins removed')
    a.emit('artifacts helper architecture='+arch+' cached=true no-guest-agent-pins=true')
    a.check(a.x('printf "%s" "$PINFOLD_ALLOW"').stdout=='','empty allow default')
    for command, screen in [('pi --provider anthropic --model claude-sonnet-4-5',('claude-sonnet','anthropic')),
        ('claude',('Claude Code','Welcome to Claude')),('codex',('OpenAI Codex','gpt-','Sign in'))]:
        tty(['box','exec',a.NAME,'--tty','--','sh','-c','cd /tmp; timeout --foreground -k 2s 8s '+command],expected=screen)
    record=json.loads(a.run(['box','inspect',a.ready['run_id']]).stdout)['launch']['record']
    a.check('harnesses' not in record,'launch has no harness versions')
    a.down()
    a.spec['env']['PINFOLD_ALLOW']='explicit-fixture'; a.up()
    a.check(a.x('printf "%s" "$PINFOLD_ALLOW"').stdout=='explicit-fixture','spec allow override')
    a.down(); a.emit('launch record no-harness=true PINFOLD_ALLOW default-and-explicit-control=true')
    project=a.W/'project'; project.mkdir()
    subprocess.run(['git','init','-q',str(project)],check=True)
    subprocess.run(['git','-C',str(project),'-c','user.name=user','-c','user.email=user@fixture.invalid','commit','--allow-empty','-qm','fixture'],check=True)
    a.E['EDITOR']='true'
    def project_run(args):
        return subprocess.run([str(a.P),*args],env=a.E,cwd=project,capture_output=True,text=True,timeout=180)
    a.check(project_run(['config','edit']).returncode==0,'project config edit')
    report=json.loads(project_run(['config']).stdout)
    pathlib.Path(report['config']).write_text('image='+json.dumps(a.IMAGE)+'\nprofile="default"\nallow=[]\ncpus=2\nmemory="1G"\nmounts=[{host=".",kind="disk"}]\n')
    report=json.loads(project_run(['config']).stdout)
    project_disks=list(report['project']['disks'].values())
    result=project_run(['pi','--version'])
    a.check(result.returncode==0 and '1.5.2' in result.stdout,'pinfold pi PATH version')
    a.emit('pinfold pi --version PATH launch passed')
    with pathlib.Path(report['config']).open('a') as f:
        f.write('\n[routes.model]\nlogin="pi"\nprovider="anthropic"\nfrom="FIXTURE_HOST_TOKEN"\nto='+json.dumps(a.routes['pi-login']['to'])+'\n')
    tty(['pi'],cwd=project,expected=('claude-','No models available'),attached=True)
    for disk in report['project']['disks'].values(): a.run(['disk','rm',disk])
    a.emit('project entrypoint home/work disks removed')
finally:
    a.down(); a.run(['disk','rm',a.DISK],required=False)
    for disk in project_disks: a.run(['disk','rm',disk],required=False)
    for s in a.servers: s.shutdown(); s.server_close()
# A fresh synthetic host login forces a refresh into a deliberately held fixture.
entered,closed=threading.Event(),threading.Event()
class Held(a.http.server.BaseHTTPRequestHandler):
    def log_message(self,*args): pass
    def do_POST(self):
        self.rfile.read(int(self.headers.get('Content-Length',0))); entered.set()
        self.connection.settimeout(35)
        try:
            if self.connection.recv(1)==b'': closed.set()
        except OSError: pass
server=a.http.server.ThreadingHTTPServer(('127.0.0.1',0),Held)
threading.Thread(target=server.serve_forever,daemon=True).start()
expired=a.enc({'alg':'none'})+'.'+a.enc({'exp':int(time.time())-10,'https://api.openai.com/auth':{'chatgpt_account_id':a.account}})+'.fixture'
a.MARKERS.append(expired.encode())
(a.W/'auth/auth.json').write_text(json.dumps({'auth_mode':'chatgpt','OPENAI_API_KEY':None,
    'tokens':{'id_token':expired,'access_token':expired,'refresh_token':a.refresh,'account_id':a.account},'last_refresh':'2020-01-01T00:00:00Z'}))
(a.W/'auth/config.toml').write_text('cli_auth_credentials_store="file"\nmodel_provider="hostile"\n[model_providers.hostile]\nname="hostile"\nbase_url="http://127.0.0.1:9"\nwire_api="responses"\n')
a.E['CODEX_REFRESH_TOKEN_URL_OVERRIDE']='http://127.0.0.1:'+str(server.server_port)+'/refresh'
spec={'name':a.NAME+'-cancel','image':a.IMAGE,'egress':{'routes':{'codex':{'login':'codex'}}}}
err=(a.W/'cancel.stderr').open('w')
p=subprocess.Popen([str(a.P),'box','up'],env=a.E,cwd=a.W,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,text=True)
try:
    p.stdin.write(json.dumps(spec)+'\n'); p.stdin.flush()
    a.check(entered.wait(30),'real helper reached held refresh fixture under hostile provider config')
    children=subprocess.run(['pgrep','-P',str(p.pid)],capture_output=True,text=True).stdout.split()
    p.send_signal(signal.SIGTERM); p.wait(timeout=30)
    a.check(closed.wait(10),'refresh connection closed after cancellation')
    alive=[]
    for child in children:
        try: os.kill(int(child),0); alive.append(child)
        except ProcessLookupError: pass
    a.check(not alive,'helper children reaped')
    a.check(not any(m in (a.W/'cancel.stderr').read_bytes() for m in a.MARKERS),'cancel diagnostic marker scan')
    a.emit('real helper hostile-config forced-provider=true held-refresh-cancel=true connection-closed=true child-reaped=true')
finally:
    if p.poll() is None: p.terminate(); p.wait(timeout=30)
    err.close(); server.shutdown(); server.server_close()
a.emit('extra acceptance cases passed; owned boxes/disks removed')
