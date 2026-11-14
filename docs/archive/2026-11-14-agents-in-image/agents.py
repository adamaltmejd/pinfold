#!/usr/bin/env python3
"""Manual image-agent fixture exchange. Uses only synthetic configuration."""
import datetime, http.server, json, os, pathlib, select, shlex, subprocess, sys, threading, time

ROOT = pathlib.Path(__file__).resolve().parents[3]
W = pathlib.Path(os.environ['PROBE_ROOT']).resolve()
if not str(W).startswith(('/tmp/pfi.', '/private/tmp/pfi.')) or len(str(W)) > 35:
    raise RuntimeError('use a short /tmp/pfi. scratch root')
LINUX = sys.platform.startswith('linux')
P = pathlib.Path(os.environ.get('PINFOLD_BIN', ROOT/'target'/('x86_64-unknown-linux-musl/debug/pinfold' if LINUX else 'debug/pinfold')))
ENV = {k:os.environ[k] for k in ('HOME','PATH','USER','LOGNAME','TMPDIR','XDG_CACHE_HOME') if k in os.environ}
ENV.update(XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'), PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
(W/'s/pinfold').mkdir(parents=True, exist_ok=True)
(W/'s/pinfold/maintenance').touch()
LOG = W/'agents.log'
PROFILE = sys.argv[1] if len(sys.argv)>1 else 'default'
NAME = 'probe-pr2-'+W.name[4:]+'-exchange'
DISK = NAME+'-home'
captures = {agent:[] for agent in ('pi','claude','codex')}
servers = []

def emit(value):
    text = str(value).replace(str(ROOT), '/home/user/pinfold').replace(str(pathlib.Path.home()), '/home/user')
    print(text, flush=True)
    with LOG.open('a') as f: f.write(text+'\n')

def run(args, timeout=120):
    if LINUX and datetime.datetime.now(datetime.timezone.utc)>=datetime.datetime(2026,11,15,0,28,tzinfo=datetime.timezone.utc):
        raise RuntimeError('lease safety cutoff reached')
    emit('$ '+shlex.join(list(map(str,args))))
    p = subprocess.run(list(map(str,args)), env=ENV, capture_output=True, text=True, timeout=timeout)
    emit(p.stdout+p.stderr+f'[exit {p.returncode}]')
    if p.returncode: raise RuntimeError('command failed')
    return p

def x(command): return run([P,'box','exec',NAME,'--','sh','-c','set -eu; '+command])

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def do_GET(self):
        self.send_response(404); self.send_header('Content-Length','0'); self.end_headers()
    def do_POST(self):
        data = json.loads(self.rfile.read(int(self.headers.get('Content-Length',0))) or '{}')
        agent = self.server.agent
        outputs = [b for m in data.get('messages',[]) if m.get('role')=='user' and isinstance(m.get('content'),list) for b in m['content'] if b.get('type')=='tool_result']
        outputs += [i for i in data.get('input',[]) if isinstance(i,dict) and i.get('type') in ('function_call_output','custom_tool_call_output')]
        done = bool(outputs)
        marker = 'fixture-'+agent+'-command-ok'
        names = [t.get('name',t.get('function',{}).get('name')) for t in data.get('tools',[])]
        row = {'path':self.path,'tools':names,'tool_result':outputs}
        captures[agent].append(row)
        emit('fixture '+agent+' '+json.dumps(row,sort_keys=True))
        if len(captures[agent])>6:
            self.send_response(500); self.send_header('Content-Length','0'); self.end_headers(); return
        if agent in ('pi','claude'):
            content = [{'type':'text','text':'Fixture finished.'}] if done else [{'type':'tool_use','id':'toolu_fixture','name':'bash' if agent=='pi' else 'Bash','input':{'command':'printf '+marker+' > /home/user/'+agent+'-fixture; cat /home/user/'+agent+'-fixture'}}]
            response = {'id':'msg_fixture','type':'message','role':'assistant','model':data.get('model','fixture'),'content':content,'stop_reason':'end_turn' if done else 'tool_use','stop_sequence':None,'usage':{'input_tokens':1,'output_tokens':1}}
            if not data.get('stream'):
                body=json.dumps(response).encode(); self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body); return
            events=[('message_start',{'type':'message_start','message':{**response,'content':[],'stop_reason':None}})]
            for i,block in enumerate(content):
                start={**block,'input':{}} if block['type']=='tool_use' else {**block,'text':''}
                delta={'type':'input_json_delta','partial_json':json.dumps(block['input'])} if block['type']=='tool_use' else {'type':'text_delta','text':block['text']}
                events += [('content_block_start',{'type':'content_block_start','index':i,'content_block':start}),('content_block_delta',{'type':'content_block_delta','index':i,'delta':delta}),('content_block_stop',{'type':'content_block_stop','index':i})]
            events += [('message_delta',{'type':'message_delta','delta':{'stop_reason':response['stop_reason'],'stop_sequence':None},'usage':{'output_tokens':1}}),('message_stop',{'type':'message_stop'})]
        else:
            code='text((await tools.exec_command({cmd:'+json.dumps('printf '+marker+' > /home/user/codex-fixture; cat /home/user/codex-fixture; ps -eo args | grep "[c]odex-code-mode-host"')+'})).output)'
            item={'id':'msg_fixture','type':'message','role':'assistant','status':'completed','content':[{'type':'output_text','text':'Fixture finished.','annotations':[]}]} if done else {'type':'custom_tool_call','call_id':'call_fixture','name':'exec','input':code}
            response={'id':'resp_fixture','object':'response','created_at':int(time.time()),'status':'completed','model':data.get('model','fixture'),'output':[item],'usage':{'input_tokens':1,'output_tokens':1,'total_tokens':2}}
            events=[('response.created',{'type':'response.created','response':{**response,'status':'in_progress','output':[]}}),('response.output_item.done',{'type':'response.output_item.done','item':item}),('response.completed',{'type':'response.completed','response':response})]
        body=''.join('event: '+event+'\ndata: '+json.dumps(payload)+'\n\n' for event,payload in events).encode()
        self.send_response(200); self.send_header('Content-Type','text/event-stream'); self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body)

files=W/'agent-fixture'; files.mkdir(exist_ok=True)
model={'id':'fixture','name':'fixture','reasoning':False,'input':['text'],'contextWindow':200000,'maxTokens':4096,'cost':{'input':0,'output':0,'cacheRead':0,'cacheWrite':0}}
(files/'models.json').write_text(json.dumps({'providers':{'fixture':{'baseUrl':'http://pi','api':'anthropic-messages','apiKey':'pinfold-placeholder','models':[model]}}}))
(files/'claude-env.json').write_text(json.dumps({'ANTHROPIC_API_KEY':'pinfold-placeholder','ANTHROPIC_BASE_URL':'http://claude'}))
(files/'codex.toml').write_text('model="gpt-5.1-codex"\nmodel_provider="fixture"\n[model_providers.fixture]\nname="fixture"\nbase_url="http://codex"\nwire_api="responses"\nrequires_openai_auth=false\nrequest_max_retries=0\nstream_max_retries=0\n[features]\ncode_mode=true\n[features.code_mode_host]\nenabled=true\ndisable_in_process_fallback=true\n')
routes={}
for agent in captures:
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler); server.agent=agent
    servers.append(server); threading.Thread(target=server.serve_forever,daemon=True).start()
    routes[agent]='127.0.0.1:'+str(server.server_port)
inventory=run([P,'disk','list','--json']).stdout
if any(json.loads(line)['name']==DISK for line in inventory.splitlines() if line): raise RuntimeError('fixture disk already exists')
spec={'name':NAME,'profile':PROFILE,'cpus':2,'memory':'1G','env':{'HOME':'/home/user'},'disks':[{'name':DISK,'guest':'/home/user'}],'mounts':[{'host':str(files),'guest':'/fixture'}],'egress':{'allow':[],'routes':routes}}
emit('spec '+json.dumps(spec))
owner=None; ready=None
try:
    owner=subprocess.Popen([str(P),'box','up'],env=ENV,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=(W/'agents-up.stderr').open('w'),text=True)
    owner.stdin.write(json.dumps(spec)+'\n'); owner.stdin.flush()
    if not select.select([owner.stdout],[],[],180)[0]: raise RuntimeError('readiness deadline')
    ready=json.loads(owner.stdout.readline()); emit(ready)
    if ready.get('event')!='ready': raise RuntimeError('box did not start')
    x('cp /fixture/models.json /home/user/.pi/agent/models.json; jq --slurpfile env /fixture/claude-env.json \'.env = $env[0]\' /home/user/.claude/settings.json > /tmp/settings; cat /tmp/settings > /home/user/.claude/settings.json; cat /fixture/codex.toml >> /home/user/.codex/config.toml')
    x('command -v pi claude codex; pi --version; claude --version; codex --version')
    commands={'pi':'pi -p --provider fixture --model fixture "Run the fixture command."','claude':'claude -p --model claude-sonnet-4-5 "Run the fixture command."','codex':'codex exec --skip-git-repo-check --json "Run the fixture command."'}
    for agent,command in commands.items():
        x('cd /tmp; timeout --foreground 90 '+command)
        outputs=[o for row in captures[agent] for o in row['tool_result']]
        observed=json.dumps(outputs)
        ok='fixture-'+agent+'-command-ok' in observed
        if agent=='codex': ok=ok and 'codex-code-mode-host' in observed and '--listen grpc+stdio://' in observed
        emit(agent+' successful-tool-exchange='+str(ok))
        if not ok: raise RuntimeError('fixture tool success not established')
        if x('cat /home/user/'+agent+'-fixture').stdout!='fixture-'+agent+'-command-ok': raise RuntimeError('tool file marker absent')
        x('if printf fixture >> "$(command -v '+agent+')"; then echo unexpected-binary-write; exit 1; else echo '+agent+'-binary-write-refused; fi')
finally:
    if owner:
        try: run([P,'box','down',NAME])
        finally:
            owner.stdin.close()
            try: owner.wait(timeout=90)
            except subprocess.TimeoutExpired: owner.terminate(); owner.wait(timeout=30)
            emit(owner.stdout.read()); emit((W/'agents-up.stderr').read_text())
    if ready and ready.get('egress_log'):
        rows=[json.loads(line) for line in pathlib.Path(ready['egress_log']).read_text().splitlines()]
        for row in rows: emit('agent-audit '+json.dumps(row,sort_keys=True))
        hosts={row.get('host') for row in rows}
        emit('version-or-plugin-host-contact='+str(bool(hosts & {'pi.dev','api.github.com','downloads.claude.ai','registry.npmjs.org','formulae.brew.sh'})))
    for server in servers: server.shutdown(); server.server_close()
    run([P,'disk','rm',DISK])
