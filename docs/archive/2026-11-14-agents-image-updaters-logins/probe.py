#!/usr/bin/env python3
"""Manual macOS probe. Only fresh state and probe-prefixed resources."""
import json, os, pathlib, shutil, subprocess, sys, tempfile, time
D = pathlib.Path(__file__).resolve().parent
ROOT = D.parents[2]
P = ROOT / 'target/debug/pinfold'
W = pathlib.Path(os.environ.get('PROBE_ROOT') or tempfile.mkdtemp(prefix='pfa.', dir='/tmp'))
ENV = {k: os.environ[k] for k in ('HOME','PATH','TMPDIR','USER','LOGNAME') if k in os.environ}
ENV.update(XDG_CONFIG_HOME=str(W/'config'), XDG_STATE_HOME=str(W/'state'), PINFOLD_NO_UPDATE_CHECK='1', RUSTC_WRAPPER='')
(W/'state/pinfold').mkdir(parents=True, exist_ok=True)
(W/'state/pinfold/maintenance').touch()
LOG = D / (os.environ.get('PROBE_LOG',sys.argv[1]) + '.log')
def scrub(s):
    return s.replace(str(pathlib.Path.home()), '/Users/user').replace(str(ROOT), '/Users/user/project')
def emit(s):
    s=scrub(str(s)); print(s, flush=True)
    with LOG.open('a') as f: f.write(s+'\n')
def run(args, timeout=120, **kw):
    emit('$ '+ ' '.join(str(a) for a in args))
    start=time.monotonic()
    try:
        p=subprocess.run([str(a) for a in args], env=ENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout, **kw)
        emit(p.stdout); emit(f'[exit {p.returncode}; seconds {time.monotonic()-start:.3f}]'); return p
    except subprocess.TimeoutExpired as e:
        emit(e.stdout.decode(errors='replace') if isinstance(e.stdout,bytes) else e.stdout or '')
        emit(f'[host timeout {timeout}s]'); raise
emit('scratch='+str(W))
if sys.argv[1] in ('build','refresh'):
    profile=W/'config/pinfold/profiles/probe-agents'; profile.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(D/'Containerfile',profile/'Containerfile')
    for label in (('refresh',) if sys.argv[1]=='refresh' else ('cold','warm')):
        emit('== '+label)
        run(['df','-g','/'])
        free=shutil.disk_usage('/').free
        if free < 12*1024**3: emit('STOP: under 12 GiB'); break
        p=run([P,'build','--profile','probe-agents']+(['--no-cache'] if sys.argv[1]=='refresh' else []), timeout=900)
        if p.returncode: break
    run(['container','image','inspect','localhost/pinfold/profile-probe-agents:latest'])
elif sys.argv[1] in ('baseline','updates','logins','inventory','claude-fixture','logins-native','logins-injected'):
    import threading, http.server, select
    IMAGE='localhost/pinfold/profile-probe-agents:latest'
    owners=[]; disks=[]; servers=[]
    import uuid
    suffix=uuid.uuid4().hex[:10]
    def up(name, env=None, mounts=None, routes=None, allow=None, home='/home/user', disk_specs=None):
        name=name+'-'+suffix
        if disk_specs:
            disk_specs=[{**d,'name':d['name']+'-'+suffix} for d in disk_specs]
        spec={'name':name,'image':IMAGE,'cpus':2,'memory':'4G','env':{'HOME':home,**(env or {})},
              'disks':disk_specs or [{'name':name+'-home','guest':home},{'name':name+'-work','guest':'/work'}],
              'mounts':mounts or [],'egress':{'allow':allow or [],'routes':routes or {}}}
        if sys.argv[1] in ('logins-native','logins-injected'): spec['harnesses']=['pi']
        inventory=run([P,'disk','list','--json'])
        if inventory.returncode: raise RuntimeError('cannot establish disk ownership')
        existing=[json.loads(line) for line in inventory.stdout.splitlines() if line.strip()]
        planned={d['name'] for d in spec['disks']}
        if any(d.get('name') in planned for d in existing): raise RuntimeError('planned disk already exists')
        emit('box spec: '+json.dumps(spec))
        p=subprocess.Popen([str(P),'box','up'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=ENV)
        owners.append((name,p)); p.stdin.write(json.dumps(spec)+'\n'); p.stdin.flush()
        ready,_,_=select.select([p.stdout],[],[],120)
        if not ready: raise RuntimeError('box readiness timeout')
        line=p.stdout.readline(); emit(line)
        obj=json.loads(line)
        if obj.get('event')!='ready': raise RuntimeError('box failed '+line)
        disks.extend(d['name'] for d in spec['disks'])
        return name,obj
    def x(box, command, timeout=150):
        return run([P,'box','exec',box,'--','sh','-c',command],timeout=timeout)
    def events(obj,label):
        path=pathlib.Path(obj['egress_log'])
        text=path.read_text() if path.exists() else ''
        (D/(label+'-egress.jsonl')).write_text(scrub(text)); emit('egress '+label+':\n'+text)
    def down(box):
        run([P,'box','down',box]);
        for name,p in owners:
            if name==box and not p.stdout.closed:
                p.stdin.close(); p.wait(timeout=60); emit(p.stdout.read()); emit(p.stderr.read()); p.stdout.close(); p.stderr.close()
    try:
        if sys.argv[1]=='inventory':
            box,obj=up('probe-agents-inventory')
            x(box,'cat /usr/local/share/probe-agents/install-time /usr/local/share/probe-agents/packages /usr/local/share/probe-agents/cache-before /usr/local/share/probe-agents/cache-after; node --version; npm --version; du -sk /usr /var /etc; cat /usr/local/share/probe-agents/install.log')
        elif sys.argv[1]=='baseline':
            box,obj=up('probe-agents-baseline')
            x(box,'cd /work; id; env | sort; cat /usr/local/share/probe-agents/install-time /usr/local/share/probe-agents/packages /usr/local/share/probe-agents/cache-before /usr/local/share/probe-agents/cache-after; du -sk /usr/local/lib/node_modules/* /usr/local/bin/node; cat /usr/local/share/probe-agents/install.log')
            for agent in ('pi','claude','codex'):
                x(box,agent+' --version')
            for command in ('pi -p "say hi" --provider anthropic --model sonnet','claude -p "say hi"','codex exec --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox "say hi"'):
                x(box,'cd /work; timeout 35 '+command)
            # Normal interactive start, no credentials and no login action.
            import pty, re, signal
            for agent in ('pi','claude','codex'):
                emit('$ box exec '+box+' --tty -- '+agent+' [18 second observation]')
                master,slave=pty.openpty()
                import fcntl, struct, termios
                fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,120,0,0))
                p=subprocess.Popen([str(P),'box','exec',box,'--tty','--',agent],stdin=slave,stdout=slave,stderr=slave,env={**ENV,'TERM':'xterm-256color'},start_new_session=True)
                os.close(slave); end=time.monotonic()+18; chunks=[]
                while time.monotonic()<end and p.poll() is None:
                    r,_,_=select.select([master],[],[],0.3)
                    if r:
                        try: chunks.append(os.read(master,65536))
                        except OSError: break
                if p.poll() is None: p.terminate()
                try:p.wait(timeout=5)
                except subprocess.TimeoutExpired:p.kill();p.wait()
                os.close(master)
                raw=b''.join(chunks).decode(errors='replace')
                (D/(agent+'-start.ansi')).write_text(scrub(raw))
                clean=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07)|\x1b.', '',raw)
                emit(clean);emit('[observation client exit '+str(p.returncode)+']')
            events(obj,'baseline')
        elif sys.argv[1]=='claude-fixture':
            import hashlib, io, tarfile
            payload=io.BytesIO()
            with tarfile.open(fileobj=payload,mode='w:gz') as archive:
                content=b'{"name":"@anthropic-ai/claude-code","version":"999.0.0"}'
                member=tarfile.TarInfo('package/package.json');member.size=len(content);archive.addfile(member,io.BytesIO(content))
            tarball=payload.getvalue(); requests=[]
            class Registry(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    requests.append({'method':self.command,'path':self.path})
                    package={'name':'@anthropic-ai/claude-code','version':'999.0.0','dist':{'tarball':'http://fixture-npm/fixture.tgz','shasum':hashlib.sha1(tarball).hexdigest()}}
                    data=tarball if self.path=='/fixture.tgz' else json.dumps({'name':package['name'],'dist-tags':{'latest':'999.0.0'},'versions':{'999.0.0':package}}).encode()
                    self.send_response(200);self.send_header('Content-Type','application/octet-stream' if self.path=='/fixture.tgz' else 'application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
                def log_message(self,*args):pass
            server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Registry);servers.append(server)
            threading.Thread(target=server.serve_forever,daemon=True).start()
            box,obj=up('probe-agents-claude-fixture',env={'npm_config_registry':'http://fixture-npm','npm_config_update_notifier':'false','CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1'},routes={'fixture-npm':'127.0.0.1:'+str(server.server_port)})
            x(box,'cd /work; timeout 75 strace -f -e trace=file -o /tmp/claude.trace claude update')
            x(box,"rg 'execve|/usr/local/.*(EROFS|EACCES)|rename.*claude|mkdir.*claude' /tmp/claude.trace | tail -70; claude --version")
            emit('fixture registry requests: '+json.dumps(requests)); events(obj,'claude-fixture')
        elif sys.argv[1]=='updates':
            allow=['api.anthropic.com','platform.claude.com','api.openai.com','auth.openai.com','chatgpt.com','openrouter.ai','opencode.ai','registry.npmjs.org','pi.dev']
            for defaults in (True,False):
                env={'PI_TELEMETRY':'0','PI_SKIP_VERSION_CHECK':'1','CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1'} if defaults else {}
                box,obj=up('probe-agents-updates-'+str(defaults).lower(),env=env,allow=allow)
                x(box,'cd /work; env | sort; npm config get prefix; npm config get cache; ls -ld /usr/local/lib/node_modules /usr/local/lib/node_modules/*; command -v pi claude codex; pi update --help; claude update --help; codex --help')
                for label,cmd in [('pi','pi update --self'),('pi-force','pi update --self --force'),('claude','claude update'),('codex','npm install -g @openai/codex@latest')]:
                    x(box,"cd /work; timeout 75 strace -f -e trace=file -o /tmp/"+label+".trace "+cmd)
                    x(box,"echo '== attempted writes'; rg 'O_WRONLY|O_RDWR|mkdir|rename|unlink|EROFS|EACCES' /tmp/"+label+".trace | tail -65; echo '== created files'; find /home/user -type f -not -path '*/_cacache/*' | sort | head -60")
                    events(obj,'updates-'+str(defaults).lower()+'-'+label)
                down(box)
        else:
            captures={}
            class Handler(http.server.BaseHTTPRequestHandler):
                def do_POST(self):
                    data=self.rfile.read(int(self.headers.get('Content-Length',0)))
                    row={'route':self.server.label,'method':self.command,'path':self.path,'headers':list(self.headers.raw_items()),'body_bytes':len(data)}
                    captures[self.server.label].append(row)
                    body=b'{"error":{"type":"invalid_request_error","message":"fixture"}}'
                    self.send_response(400);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
                do_GET=do_POST
                def log_message(self,*a):pass
            routes={}
            for label in ('anthropic','claude','codex'):
                server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler);server.label=label;captures[label]=[]
                servers.append(server);threading.Thread(target=server.serve_forever,daemon=True).start()
                routes[label]='127.0.0.1:'+str(server.server_port)
            files=W/'files';files.mkdir(exist_ok=True)
            for name in ('pi','claude','codex'): (files/name).mkdir(exist_ok=True)
            (files/'pi/pinfold-login.ts').write_text('export default function(pi) { pi.registerProvider("anthropic", {baseUrl:"http://anthropic",apiKey:"pinfold-placeholder"}); }\n')
            (files/'claude/managed-settings.json').write_text(json.dumps({'env':{'ANTHROPIC_BASE_URL':'http://claude','CLAUDE_CODE_OAUTH_TOKEN':'pinfold-placeholder','ANTHROPIC_API_KEY':''}}))
            (files/'codex/config.toml').write_text('model_provider = "pinfold"\n[model_providers.pinfold]\nname = "pinfold"\nbase_url = "http://codex/backend-api/codex"\nwire_api = "responses"\nrequires_openai_auth = false\n')
            # A separate fresh disk at the agent directory gives its root to the
            # box user before its extensions mount; no prior disk boot or root exec.
            if sys.argv[1]=='logins-injected':
                ENV.update(FAKE_PI_KEY='fixture-pi-key',FAKE_CLAUDE_TOKEN='fixture-claude-token',FAKE_CODEX_ACCOUNT='fixture-account')
                helper_lines=(D/'helper-fixture-output-enoent.txt').read_text().splitlines()
                ENV['FAKE_CODEX_TOKEN']=next(json.loads(line[8:])['result']['authToken'] for line in helper_lines if line.startswith('receive ') and json.loads(line[8:]).get('result',{}).get('authMethod')=='chatgpt')
                headers={'anthropic':{'x-api-key':{'from':'FAKE_PI_KEY'}},'claude':{'Authorization':{'from':'FAKE_CLAUDE_TOKEN','prefix':'Bearer '}},'codex':{'Authorization':{'from':'FAKE_CODEX_TOKEN','prefix':'Bearer '},'ChatGPT-Account-ID':{'from':'FAKE_CODEX_ACCOUNT'}}}
                routes={label:{'to':'http://'+target,'headers':headers[label]} for label,target in routes.items()}
            box,obj=up('probe-agents-logins',env={'ANTHROPIC_API_KEY':'pinfold-placeholder'},routes=routes,
                disk_specs=[{'name':'probe-agents-login-home','guest':'/home/user'},{'name':'probe-agents-login-agent','guest':'/home/user/.pi/agent'},{'name':'probe-agents-login-work','guest':'/work'}],
                mounts=[{'host':str(files/'pi'),'guest':'/home/user/.pi/agent/extensions'},{'host':str(files/'claude'),'guest':'/etc/claude-code'},{'host':str(files/'codex'),'guest':'/etc/codex'}])
            x(box,'cd /work; id; env | sort; ls -ldn /home/user /home/user/.pi /home/user/.pi/agent /home/user/.pi/agent/extensions; for d in /home/user/.pi/agent/extensions /etc/claude-code /etc/codex; do touch "$d/write-probe"; done; true')
            commands={'pi':'pi -p "say hi" --provider anthropic --model sonnet','claude':'claude -p "say hi"','codex':'codex exec --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox "say hi"'}
            if sys.argv[1] in ('logins-native','logins-injected'): commands['pi']=commands['pi'].replace('pi -p','/opt/pinfold/pi/pi -p')
            for label,cmd in commands.items():
                x(box,'cd /work; timeout 75 '+cmd)
                emit('== fixture requests after '+label+'\n'+json.dumps(captures,indent=2))
            (D/(sys.argv[1]+'-headers.json')).write_text(json.dumps(captures,indent=2)+'\n')
            for name in ('pi','claude','codex'):
                shutil.copytree(files/name,D/('files-'+name),dirs_exist_ok=True)
            events(obj,sys.argv[1])
            x(box,'ls -ldn /home/user/.pi/agent/sessions; cat /etc/claude-code/managed-settings.json /etc/codex/config.toml /home/user/.pi/agent/extensions/pinfold-login.ts')
    finally:
        for box,p in owners:
            if p.poll() is None: down(box)
        for disk in dict.fromkeys(disks): run([P,'disk','rm',disk])
        for server in servers: server.shutdown();server.server_close()
        run([P,'disk','list','--json'])
elif sys.argv[1]=='cleanup':
    run([P,'box','list','--label','dev.pinfold.profile=probe-agents'])
    run([P,'disk','list','--json'])
    emit('$ container image list --format json [only probe-agents refs printed]')
    result=subprocess.run(['container','image','list','--format','json'],env=ENV,text=True,capture_output=True,check=True)
    images=json.loads(result.stdout)
    for entry in images:
        if 'localhost/pinfold/profile-probe-agents:' in json.dumps(entry): emit(json.dumps(entry,sort_keys=True))
    emit('$ container system status [status only]')
    result=subprocess.run(['container','system','status'],env=ENV,text=True,capture_output=True,check=True)
    emit('\n'.join(line for line in result.stdout.splitlines() if line.startswith('status ')))
    run(['git','status','--short'],cwd=ROOT)
