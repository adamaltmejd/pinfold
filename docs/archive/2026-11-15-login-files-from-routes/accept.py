#!/usr/bin/env python3
"""Manual acceptance collector. Usage: P=/path/to/pinfold IMAGE=local:tag python3 accept.py."""
import base64, datetime, http.server, json, os, pathlib, select, shlex
import subprocess, tempfile, threading, time, concurrent.futures
P = pathlib.Path(os.environ['P']).resolve()
IMAGE = os.environ.get('IMAGE', 'localhost/pinfold/profile-default:latest')
W = pathlib.Path(tempfile.mkdtemp(prefix='pf3.', dir='/tmp'))
E = {k: os.environ[k] for k in ('HOME', 'PATH', 'TMPDIR', 'XDG_CACHE_HOME') if k in os.environ}
E.update(USER='user', LOGNAME='user', XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'),
         CODEX_HOME=str(W/'auth'), PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
for d in ('auth', 's/pinfold'): (W/d).mkdir(parents=True)
(W/'s/pinfold/maintenance').touch()
(W/'auth/config.toml').write_text('cli_auth_credentials_store="file"\n')
static, account, refresh = ['HOSTONLY-'+k+'-'+W.name[4:] for k in ('static', 'account', 'refresh')]
def enc(v): return base64.urlsafe_b64encode(json.dumps(v).encode()).rstrip(b'=').decode()
jwt = enc({'alg':'none'})+'.'+enc({'exp':int(time.time())+86400, 'email':'fixture@example.invalid',
    'https://api.openai.com/auth':{'chatgpt_account_id':account,'chatgpt_plan_type':'plus'}})+'.fixture'
(W/'auth/auth.json').write_text(json.dumps({'auth_mode':'chatgpt','OPENAI_API_KEY':None,
    'tokens':{'id_token':jwt,'access_token':jwt,'refresh_token':refresh,'account_id':account},
    'last_refresh':datetime.datetime.now(datetime.timezone.utc).isoformat()}))
E['FIXTURE_HOST_TOKEN'] = static
MARKERS = [v.encode() for v in (static, account, refresh, jwt)]
NAME = 'probe-pr3-accept-'+W.name[4:]
DISK = NAME+'-home'
owner, ready, home, diagnostics, captures, servers = None, None, '/home/user', [], {}, []
def check(ok, label):
    if not ok: raise RuntimeError(label)
def clean(v):
    s = str(v).replace(str(W), '$W').replace(str(pathlib.Path.home()), '/home/user')
    for v in MARKERS: s = s.replace(v.decode(), '[HOST-MARKER]')
    return s
def emit(s): print(clean(s), flush=True)
def run(args, required=True, timeout=150):
    p = subprocess.run([str(P), *args], env=E, cwd=W, text=True, capture_output=True,
                       errors='replace', timeout=timeout)
    diagnostics.extend([p.stdout.encode(), p.stderr.encode()])
    if required and p.returncode:
        emit((p.stdout+p.stderr)[-2200:]); raise RuntimeError('CLI failure: '+shlex.join(args[:4]))
    return p
def x(command, required=True):
    return run(['box','exec',NAME,'--','sh','-c','set -eu; '+command], required)
def up():
    global owner, ready
    start = time.monotonic()
    owner = subprocess.Popen([str(P),'box','up'], env=E, stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=(W/'up.stderr').open('w'), text=True)
    owner.stdin.write(json.dumps(spec)+'\n'); owner.stdin.flush()
    check(select.select([owner.stdout], [], [], 180)[0], 'readiness deadline')
    row = owner.stdout.readline()
    check(bool(row), 'owner ended before readiness')
    ready = json.loads(row)
    if ready.get('event') != 'ready':
        emit(ready); emit((W/'up.stderr').read_text()); raise RuntimeError('box not ready')
    emit('ready generated-mounts='+str(sum(any(isinstance(v,dict) and v.get('login')==agent for v in spec['egress']['routes'].values()) for agent in ('pi','claude','codex')))+
         ' startup-seconds='+str(round(time.monotonic()-start,3)))
def down():
    global owner
    if owner:
        run(['box','down',NAME], required=False)
        owner.stdin.close()
        try: owner.wait(timeout=90)
        except subprocess.TimeoutExpired: owner.terminate(); owner.wait(timeout=30)
        diagnostics.extend([owner.stdout.read().encode(), (W/'up.stderr').read_bytes()])
        owner = None
def anthropic(data, done, tool, command, marker):
    block = {'type':'text','text':marker} if done else {'type':'tool_use','id':'toolu_fixture',
        'name':tool,'input':{'command':command}}
    response = {'id':'msg_fixture','type':'message','role':'assistant','model':data.get('model','fixture'),
        'content':[block],'stop_reason':'end_turn' if done else 'tool_use','stop_sequence':None,
        'usage':{'input_tokens':1,'output_tokens':1}}
    events = [('message_start',{'type':'message_start','message':{**response,'content':[],'stop_reason':None}})]
    start = {**block,'text':''} if done else {**block,'input':{}}
    delta = {'type':'text_delta','text':marker} if done else {'type':'input_json_delta','partial_json':json.dumps(block['input'])}
    events += [('content_block_start',{'type':'content_block_start','index':0,'content_block':start}),
        ('content_block_delta',{'type':'content_block_delta','index':0,'delta':delta}),
        ('content_block_stop',{'type':'content_block_stop','index':0}),
        ('message_delta',{'type':'message_delta','delta':{'stop_reason':response['stop_reason'],'stop_sequence':None},'usage':{'output_tokens':1}}),
        ('message_stop',{'type':'message_stop'})]
    return response, events
def responses(data, done, tool, command, marker):
    custom = any(t.get('type')=='custom' and t.get('name')==tool for t in data.get('tools',[]))
    code = 'text((await tools.exec_command({cmd:'+json.dumps(command)+'})).output)'
    item = {'id':'msg_fixture','type':'message','role':'assistant','status':'completed',
        'content':[{'type':'output_text','text':marker,'annotations':[]}]} if done else (
        {'id':'fc_fixture','type':'custom_tool_call','call_id':'call_fixture','name':tool,'input':code} if custom else
        {'id':'fc_fixture','type':'function_call','call_id':'call_fixture','name':tool,'arguments':json.dumps({'command':command})})
    response = {'id':'resp_fixture','object':'response','created_at':int(time.time()),'status':'completed',
        'model':data.get('model','fixture'),'output':[item], 'usage':{'input_tokens':1,'output_tokens':1,'total_tokens':2,
        'input_tokens_details':{'cached_tokens':0},'output_tokens_details':{'reasoning_tokens':0}}}
    events = [('response.created',{'type':'response.created','response':{**response,'status':'in_progress','output':[]}}),
        ('response.output_item.added',{'type':'response.output_item.added','output_index':0,'item':{**item,**({'content':[]} if done else {'arguments':''} if not custom else {'input':''})}})]
    if done:
        events += [('response.content_part.added',{'type':'response.content_part.added','output_index':0,'content_index':0,'part':{'type':'output_text','text':'','annotations':[]}}),
            ('response.output_text.delta',{'type':'response.output_text.delta','output_index':0,'content_index':0,'delta':marker})]
    elif not custom:
        events += [('response.function_call_arguments.delta',{'type':'response.function_call_arguments.delta','output_index':0,'delta':item['arguments']})]
    if not done and not custom:
        events += [('response.function_call_arguments.done',{'type':'response.function_call_arguments.done','output_index':0,'arguments':item['arguments']})]
    events += [('response.output_item.done',{'type':'response.output_item.done','output_index':0,'item':item}),
        ('response.completed',{'type':'response.completed','response':response})]
    return response, events
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def do_GET(self):
        self.send_response(404); self.send_header('Content-Length','0'); self.end_headers()
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length',0)))
        if self.headers.get('Content-Encoding') == 'zstd':
            body = subprocess.run(['zstd','-d','-q','-c'], input=body, capture_output=True, check=True).stdout
        data = json.loads(body or '{}')
        srv = self.server
        outputs = [b for m in data.get('messages',[]) if m.get('role')=='user' and isinstance(m.get('content'),list)
            for b in m['content'] if b.get('type')=='tool_result']
        outputs += [m for m in data.get('messages',[]) if m.get('role')=='tool']
        outputs += [i for i in data.get('input',[]) if isinstance(i,dict) and i.get('type') in ('function_call_output','custom_tool_call_output')]
        declared = data.get('tools',[]) + [t for i in data.get('input',[]) if isinstance(i,dict) and i.get('type')=='additional_tools' for t in i.get('tools',[])]
        names = [t.get('name',t.get('function',{}).get('name')) for t in declared]
        tool = next((n for n in names if n and n.lower()=='bash'), 'exec')
        auth = self.headers.get('Authorization') == 'Bearer '+(jwt if srv.host_codex else static)
        if srv.api_key: auth = self.headers.get('x-api-key') == static
        account_ok = not srv.host_codex or self.headers.get('ChatGPT-Account-ID') == account
        captures[srv.label].append({'outputs':outputs,'auth':auth and account_ok,'path':self.path,'tools':names,'host_marker':any(m in str(list(self.headers.items())).encode() for m in MARKERS)})
        done = bool(outputs)
        if not done and getattr(srv,'barrier',None): srv.barrier.wait(timeout=30)
        if self.path.endswith('/chat/completions'):
            delta = {'content':srv.marker} if done else {'tool_calls':[{'index':0,'id':'call_fixture','type':'function',
                'function':{'name':tool,'arguments':json.dumps({'command':srv.command})}}]}
            response = {'id':'chat_fixture','object':'chat.completion','created':int(time.time()),'model':data.get('model'),
                'choices':[{'index':0,'message':{'role':'assistant',**delta},'finish_reason':'stop' if done else 'tool_calls'}]}
            payload = {**response,'object':'chat.completion.chunk','choices':[{'index':0,'delta':delta,'finish_reason':None}]}
            end = {**payload,'choices':[{'index':0,'delta':{},'finish_reason':'stop' if done else 'tool_calls'}]}
            body = ('data: '+json.dumps(payload)+'\n\ndata: '+json.dumps(end)+'\n\ndata: [DONE]\n\n').encode()
        else:
            renderer = anthropic if '/messages' in self.path else responses
            response, events = renderer(data, done, tool, srv.command, srv.marker)
            body = ''.join('event: '+event+'\ndata: '+json.dumps(payload)+'\n\n' for event,payload in events).encode()
        if not data.get('stream'): body = json.dumps(response).encode()
        self.send_response(200); self.send_header('Content-Type','text/event-stream' if data.get('stream') else 'application/json')
        self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body)
def route(label, agent, provider=None):
    srv = http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
    srv.label, srv.host_codex = label, agent=='codex' or provider=='openai-codex'
    srv.api_key = provider=='anthropic'
    srv.marker, srv.command = '', ''
    captures[label] = []; servers.append(srv)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    value = {'login':agent,'to':'http://127.0.0.1:'+str(srv.server_port)}
    if provider: value['provider'] = provider
    if not srv.host_codex: value['from'] = 'FIXTURE_HOST_TOKEN'
    return srv, value
routes, fixtures = {}, {}
spec = {'name':NAME,'image':IMAGE,'cpus':2,'memory':'1G','env':{'HOME':home},
    'disks':[{'name':DISK,'guest':'/home/user'}],'egress':{'allow':[],'routes':routes}}
def exchange(label, command, auth=True, cwd="/tmp"):
    srv = fixtures[label]; srv.marker = 'fixture-'+label+'-command-ok'
    srv.command = 'printf '+srv.marker+' > "$HOME/'+label+'-fixture"; cat "$HOME/'+label+'-fixture"'
    captures[label].clear()
    result = x('cd '+shlex.quote(cwd)+'; timeout --foreground 100 '+command, required=False)
    if result.returncode: emit((result.stdout+result.stderr)[-1800:])
    check(result.returncode==0, label+' agent exit')
    if not any(srv.marker in json.dumps(r['outputs']) for r in captures[label]): emit('fixture-debug '+json.dumps(captures[label])+' '+result.stdout[-1000:]+result.stderr[-1000:])
    check(any(srv.marker in json.dumps(r['outputs']) for r in captures[label]), label+' fixture saw successful tool output')
    check(all(r['auth']==auth for r in captures[label]), label+' expected host authentication')
    check(x('cat "$HOME/'+label+'-fixture"').stdout==srv.marker, label+' tool host-observed file')
    emit(label+' successful-tool-exchange=true host-auth='+str(auth)+' requests='+str(len(captures[label])))
pi_command = lambda provider, model: 'pi -p --provider '+provider+' --model '+model+' "Run fixture tool."'
def boundaries():
    x("test -f \"$HOME/.pi/agent/settings.json\" || printf '%s' '{\"defaultProjectTrust\":\"always\"}' > \"$HOME/.pi/agent/settings.json\"")
    x("printf '%s' "+shlex.quote('{"anthropic":{"type":"api_key","key":"fixture-stored-guest"}}')+' > "$HOME/.pi/agent/auth.json"')
    exchange('pi-anthropic',pi_command('anthropic','claude-sonnet-4-5'))
    exchange('pi-anthropic',pi_command('anthropic','claude-sonnet-4-5')+' --api-key fixture-explicit-guest')
    x('rm "$HOME/.pi/agent/auth.json"')
    server,value=route('hostile-project','pi','anthropic'); fixtures['hostile-project']=server
    routes['bypass']='127.0.0.1:'+str(server.server_port)
    down(); up()
    extension='export default function(pi) { pi.registerProvider("anthropic",{baseUrl:"http://bypass",apiKey:"fixture-project-guest"}); }'
    x('mkdir -p "$HOME/project/.pi/extensions"; printf %s '+shlex.quote(extension)+' > "$HOME/project/.pi/extensions/hostile.ts"')
    exchange('hostile-project',pi_command('anthropic','claude-sonnet-4-5'),False,home+'/project')
    check(not any(r['host_marker'] for r in captures['hostile-project']),'project override bypass has no host authentication marker')
    emit('stored-and-explicit guest keys replaced by login route; hostile project override completed with no host authentication')
def allow_controls():
    routes.clear(); spec['egress']['allow']=['api.github.com','example.com']; spec['env'].pop('PINFOLD_ALLOW',None)
    up(); check(x('printf "%s" "$PINFOLD_ALLOW"').stdout=='api.github.com,example.com','allow default without login routes'); down()
    spec['env']['PINFOLD_ALLOW']='explicit-allow'
    up(); check(x('printf "%s" "$PINFOLD_ALLOW"').stdout=='explicit-allow','explicit allow without login routes'); down()
    emit('PINFOLD_ALLOW without harness/login routes: comma-list default and explicit override passed')
def main():
    global home
    for label, agent, provider in [('claude-login','claude',None),('codex-login','codex',None),
        ('pi-anthropic','pi','anthropic'),('pi-openai','pi','openai'),('pi-router','pi','openrouter'),('pi-codex','pi','openai-codex')]:
        fixtures[label], routes[label] = route(label, agent, provider)
    try:
        up()
        check(x('getent passwd "$(id -u)"', required=False).returncode!=0, 'numeric user has no passwd entry')
        emit('fresh-single-home explicit-image no-profile absent-passwd=true')
        versions = x('pi --version; claude --version; codex --version').stdout.strip().replace('\n','; ')
        emit('image agents: '+versions)
        x('mkdir -p "$HOME/.claude" "$HOME/.codex"; '+
          "printf '%s' "+shlex.quote('{"permissions":{"defaultMode":"bypassPermissions"},"skipDangerousModePermissionPrompt":true,"sandbox":{"enabled":false}}')+' > "$HOME/.claude/settings.json"; '+
          "printf '%s' "+shlex.quote('approval_policy="never"\nsandbox_mode="danger-full-access"\nmodel="gpt-5.1-codex"\ncheck_for_update_on_startup=false\n[features]\ncode_mode=true\n[features.code_mode_host]\nenabled=true\ndisable_in_process_fallback=true\n')+' > "$HOME/.codex/config.toml"; '+
          "printf '%s' "+shlex.quote('{"quietStartup":true,"defaultProjectTrust":"always"}')+' > "$HOME/.pi/agent/settings.json"')
        extension = 'import {writeFileSync} from "node:fs"; export default function(pi) { pi.on("session_start",()=>{writeFileSync(process.env.HOME+"/extension.loaded","sibling-loaded")}) }'
        x("printf '%s' "+shlex.quote(extension)+' > "$HOME/.pi/agent/extensions/sibling.ts"')
        original = x('cat "$HOME/.pi/agent/settings.json"; cat "$HOME/.claude/settings.json"; cat "$HOME/.codex/config.toml"').stdout
        for path in ('/etc/claude-code/managed-settings.json','/etc/codex/config.toml',home+'/.pi/agent/extensions/pinfold-login/index.ts'):
            x('if printf x >> '+shlex.quote(path)+' 2>/dev/null; then exit 1; fi')
        x('printf sibling-writable > "$HOME/sibling"')
        emit('generated-files readonly=true sibling-home writable=true')
        for label, command in [('pi-anthropic',pi_command('anthropic','claude-sonnet-4-5')),
            ('claude-login','claude -p --model claude-sonnet-4-5 "Run fixture tool."'),
            ('codex-login','codex exec --skip-git-repo-check --json "Run fixture tool."'),
            ('pi-openai',pi_command('openai','gpt-4o')),
            ('pi-router',pi_command('openrouter','anthropic/claude-sonnet-4.5')),
            ('pi-codex',pi_command('openai-codex','gpt-5.1-codex'))]: exchange(label, command)
        boundaries()
        barrier = threading.Barrier(2)
        fixtures['codex-login'].barrier = fixtures['pi-codex'].barrier = barrier
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            futures = [pool.submit(exchange,'codex-login','codex exec --skip-git-repo-check --json "Concurrent fixture."'), pool.submit(exchange,'pi-codex',pi_command('openai-codex','gpt-5.1-codex'))]
            for future in futures: future.result()
        fixtures['codex-login'].barrier = fixtures['pi-codex'].barrier = None
        emit('simultaneous Codex and Pi fixture exchanges passed; requests synchronized at host fixture')
        check(x('cat "$HOME/extension.loaded"').stdout=='sibling-loaded', 'sibling extension loaded')
        child = fixtures['pi-anthropic']; child.marker = 'fixture-child-pi'; child.command = 'env > "$HOME/child.env"; printf fixture-child-pi > "$HOME/child-pi"; cat "$HOME/child-pi"'
        parent = fixtures['claude-login']; parent.marker = 'fixture-child-claude'
        parent.command = pi_command('anthropic','claude-sonnet-4-5')+'; printf fixture-child-claude'
        captures['pi-anthropic'].clear(); captures['claude-login'].clear()
        check(x('cd /tmp; timeout --foreground 120 claude -p --model claude-sonnet-4-5 "Launch child."').returncode==0, 'parent agent exits')
        check(any(child.marker in json.dumps(r['outputs']) for r in captures['pi-anthropic']), 'child completed tool round trip')
        check(any(parent.marker in json.dumps(r['outputs']) for r in captures['claude-login']), 'parent saw child exit')
        inherited = x('cat "$HOME/child.env"').stdout.encode()
        check(b'PINFOLD_ALLOW=' in inherited and b'http_proxy=' in inherited, 'child inherited transport env')
        check(not any(m in inherited for m in MARKERS), 'child has no host markers')
        emit('agent-launches-agent=true inherited-proxy-and-allow=true host-markers-absent=true sibling-extension-loaded=true')
        scan = x('env; cat /etc/claude-code/managed-settings.json /etc/codex/config.toml "$HOME/.pi/agent/extensions/pinfold-login/index.ts"; tar -cf - -C "$HOME" . | base64 -w0').stdout
        plain, encoded = scan.rsplit('\n',1) if '\n' in scan else ('',scan)
        guest_bytes = plain.encode()+base64.b64decode(encoded)
        check(not any(m in guest_bytes for m in MARKERS), 'guest home/env/generated-files marker scan')
        down()
        routes['pi-renamed'] = routes.pop('pi-anthropic'); routes.pop('pi-router')
        up()
        generated = x('cat "$HOME/.pi/agent/extensions/pinfold-login/index.ts"').stdout
        check('pi-renamed' in generated and 'pi-anthropic' not in generated and 'pi-router' not in generated, 'renamed and removed routes absent')
        check(x('cat "$HOME/.pi/agent/settings.json"; cat "$HOME/.claude/settings.json"; cat "$HOME/.codex/config.toml"').stdout==original, 'user settings retained')
        exchange('pi-anthropic',pi_command('anthropic','claude-sonnet-4-5'))
        down()
        srv, subscription = route('pi-subscription','pi','anthropic-subscription')
        fixtures['pi-subscription'] = srv; routes.clear(); routes['subscription-login'] = subscription
        up(); exchange('pi-subscription',pi_command('anthropic','claude-sonnet-4-5')); down()
        routes.clear(); up()
        x('test ! -e /etc/claude-code/managed-settings.json; test ! -e /etc/codex/config.toml; test ! -e "$HOME/.pi/agent/extensions/pinfold-login/index.ts"')
        down(); emit('reused-home settings-preserved=true renamed-and-removed-routes-absent=true')
        home = '/home/user/nonstandard'; spec['env']['HOME'] = home
        routes['nonstandard-login'] = subscription
        up(); exchange('pi-subscription',pi_command('anthropic','claude-sonnet-4-5')); down()
        emit('nonstandard-HOME writable-ancestor-disk=true successful-tool-exchange=true')
        allow_controls()
        for path in (W/'s').rglob('*'):
            if path.is_file() and path.suffix not in ('.ext4','.img'):
                with path.open('rb') as f:
                    previous = b''
                    while chunk := f.read(65536):
                        check(not any(m in previous+chunk for m in MARKERS), 'state/audit marker scan')
                        previous = chunk[-max(map(len,MARKERS)):]
        check(not any(m in b''.join(diagnostics) for m in MARKERS), 'diagnostic marker scan')
        emit('guest-home-env-generated-files state-audit diagnostics host-marker-scans=true; all acceptance cases passed')
    finally:
        down()
        for server in servers: server.shutdown(); server.server_close()
        removed = run(['disk','rm',DISK], required=False)
        emit('owned box stopped; home-disk-cleanup-exit='+str(removed.returncode)+'; scratch='+str(W))
if __name__ == '__main__': main()
