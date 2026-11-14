#!/usr/bin/env python3
"""Diagnostic only. Existing image; fresh scratch homes; no external network."""
import hashlib
import http.server
import json
import os
from pathlib import Path
import re
import shlex
import socketserver
import subprocess as sp
import sys
import tempfile
import threading
import time
import uuid

D = Path(__file__).resolve().parent
ROOT = D.parents[2]
W = Path(tempfile.mkdtemp(prefix='pfg7.', dir='/tmp')).resolve()
ENV = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'PINFOLD_'))}
ENV.update(GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1',
           GIT_AUTHOR_NAME='user', GIT_AUTHOR_EMAIL='user@fixture.invalid',
           GIT_COMMITTER_NAME='user', GIT_COMMITTER_EMAIL='user@fixture.invalid',
           XDG_CONFIG_HOME=str(W/'config'), XDG_STATE_HOME=str(W/'state'),
           PINFOLD_NO_UPDATE_CHECK='1')
(W/'state/pinfold').mkdir(parents=True, exist_ok=True)
(W/'state/pinfold/maintenance').touch()
IMAGE = 'localhost/pinfold/profile-probe-agents:latest'
NAME = 'probe-git7-' + uuid.uuid4().hex[:8]
MODE = sys.argv[1]
LOG = D/(os.environ.get('PROBE_LOG',MODE)+'.txt')
LOG.write_text('')
def scrub(s):
    return str(s).replace(str(Path.home()), '/Users/user').replace(str(ROOT), '/Users/user/project').replace(str(W), '$W')
def emit(s):
    s = scrub(s)
    print(s, flush=True)
    with LOG.open('a') as f:
        f.write(s+'\n')
def run(args, timeout=120, quiet=False, **kw):
    p = sp.run(list(map(str, args)), env=ENV, capture_output=True, text=True, timeout=timeout, **kw)
    if not quiet:
        emit('$ '+shlex.join(list(map(str, args)))+'\n'+p.stdout+p.stderr+f'[exit {p.returncode}]')
    return p
def git(repo, *args):
    p = run(['git', '-C', repo, *args], quiet=True)
    if p.returncode:
        raise RuntimeError(p.stderr)
    return p.stdout.strip()
def x(cmd, quiet=False, timeout=120):
    return run(['container', 'exec', '-w', '/work', NAME, 'sh', '-c', cmd], quiet=quiet, timeout=timeout)
def fresh(repo):
    repo.mkdir(parents=True)
    git(repo, 'init', '-q', '-b', 'main')
    (repo/'a').write_text('base\n')
    git(repo, 'add', 'a')
    git(repo, 'commit', '-qm', 'base')
def snapshot(repo, metadata=False):
    base = repo/'.git' if metadata else repo
    return {str(p.relative_to(base)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in base.rglob('*') if p.is_file() and (metadata or '.git' not in p.relative_to(base).parts)}
TEXT = ('This project is a writable host share. Its .git directory is read-only. '
        'Edit working-tree files only. Only the host stages, commits, stashes, switches '
        'branches, fetches, and runs Git maintenance. Report requested Git writes to '
        'the host. After a host commit, Git reads may be stale for about a second; '
        'a failed read is not evidence that history is absent.')

# A fixture HTTP server on the host is reached only through this Unix socket.
requests = []
class UnixHTTP(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def do_GET(self):
        self.send_response(404); self.end_headers()
    def do_POST(self):
        data = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or '{}')
        label = self.path.split('/')[1]
        (W/(label+'-request-'+str(len(requests))+'.json')).write_text(json.dumps(data))
        tools = data.get('tools', [])
        names = [t.get('name', t.get('function', {}).get('name')) for t in tools]
        body = json.dumps(data)
        markers = [m for m in ['PROJECT_AGENTS', 'PROJECT_CLAUDE', 'PI_APPEND', 'CODEX_HOME_AGENTS', 'CLAUDE_APPEND'] if m in body]
        outputs = []
        for m in data.get('messages', []):
            if m.get('role') == 'user' and isinstance(m.get('content'), list):
                outputs += [b for b in m['content'] if b.get('type') == 'tool_result']
        outputs += [i for i in data.get('input', []) if isinstance(i, dict) and i.get('type') == 'function_call_output']
        row = {'agent': label, 'path': self.path, 'tools': names, 'markers': markers, 'tool_results': outputs}
        requests.append(row)
        emit('fixture '+json.dumps(row))
        if MODE in ('nomodel','interactive'):
            b = b'{"error":{"type":"invalid_request_error","message":"fixture: no model"}}'
            self.send_response(400); self.send_header('Content-Length', str(len(b))); self.end_headers(); self.wfile.write(b)
            return
        done = bool(outputs)
        if label in ('pi', 'claude'):
            tool = 'bash' if label == 'pi' else 'Bash'
            content = [{'type':'text','text':'Fixture finished.'}] if done else [{'type':'tool_use','id':'toolu_fixture','name':tool,'input':{'command':'git commit -m fixture'}}]
            response = {'id':'msg_fixture','type':'message','role':'assistant','model':data.get('model','fixture'), 'content':content,'stop_reason':'end_turn' if done else 'tool_use','stop_sequence':None,'usage':{'input_tokens':1,'output_tokens':1}}
            if not data.get('stream'):
                b=json.dumps(response).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b);return
            events = [('message_start',{'type':'message_start','message':{**response,'content':[],'stop_reason':None}})]
            for i, block in enumerate(content):
                start = {**block, 'input':{}} if block['type']=='tool_use' else {**block,'text':''}
                delta = {'type':'input_json_delta','partial_json':json.dumps(block['input'])} if block['type']=='tool_use' else {'type':'text_delta','text':block['text']}
                events += [('content_block_start',{'type':'content_block_start','index':i,'content_block':start}),('content_block_delta',{'type':'content_block_delta','index':i,'delta':delta}),('content_block_stop',{'type':'content_block_stop','index':i})]
            events += [('message_delta',{'type':'message_delta','delta':{'stop_reason':response['stop_reason'],'stop_sequence':None},'usage':{'output_tokens':1}}),('message_stop',{'type':'message_stop'})]
        else:
            tool = next((n for n in names if n in ('exec_command','shell_command','shell')), 'exec_command')
            args = {'cmd':'git commit -m fixture','max_output_tokens':1000} if tool=='exec_command' else {'command':'git commit -m fixture'}
            if tool=='shell': args={'command':['sh','-c','git commit -m fixture']}
            item = {'id':'msg_fixture','type':'message','role':'assistant','status':'completed','content':[{'type':'output_text','text':'Fixture finished.','annotations':[]}]} if done else {'id':'fc_fixture','type':'function_call','call_id':'call_fixture','name':tool,'arguments':json.dumps(args),'status':'completed'}
            response = {'id':'resp_fixture','object':'response','created_at':int(time.time()),'status':'completed','model':data.get('model','fixture'),'output':[item],'usage':{'input_tokens':1,'output_tokens':1,'total_tokens':2}}
            events=[('response.created',{'type':'response.created','response':{**response,'status':'in_progress','output':[]}}),('response.output_item.added',{'type':'response.output_item.added','output_index':0,'item':{**item,'arguments':''} if not done else item})]
            if not done:
                events += [('response.function_call_arguments.delta',{'type':'response.function_call_arguments.delta','item_id':item['id'],'output_index':0,'delta':item['arguments']})]
            events += [('response.output_item.done',{'type':'response.output_item.done','output_index':0,'item':item}),('response.completed',{'type':'response.completed','response':response})]
        b=''.join('event: '+event+'\ndata: '+json.dumps(payload)+'\n\n' for event,payload in events).encode()
        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b)

repo = W/'repo'
fresh(repo)
(repo/'AGENTS.md').write_text('PROJECT_AGENTS\n'+TEXT+'\n')
(repo/'CLAUDE.md').write_text('PROJECT_CLAUDE\n'+TEXT+'\n')
(repo/'.pi').mkdir()
(repo/'.pi/APPEND_SYSTEM.md').write_text('PI_APPEND\n'+TEXT+'\n')
git(repo, 'add', '.'); git(repo, 'commit', '-qm', 'instructions')
repos = [repo]
if MODE == 'git':
    for case in ('status','add','commit','stash','checkout','fetch','gc'):
        r=W/'cases'/case; fresh(r); repos.append(r)
        git(r, 'branch', 'other')
        (r/'a').write_text('modified\n')
        (r/'untracked').write_text('untracked\n')
        if case == 'commit': git(r,'add','a')
        if case == 'fetch': git(r,'remote','add','origin','/probe/repo')
        if case == 'gc': git(r,'config','gc.auto','1')
if MODE == 'timing':
    large=W/'large'; fresh(large); repos.append(large)
    for i in range(100000):
        p=large/f'd{i//1000:03}'/f'f{i:06}';p.parent.mkdir(exist_ok=True);p.write_text('x\n')
    git(large,'add','.');git(large,'commit','-qm','100k files')
server=None
started=False
try:
    args=['container','run','-d','--name',NAME,'--network','none','--read-only','--cap-drop','ALL','--user',f'{os.getuid()}:{os.getgid()}','--cpus','4','--memory','4G','--tmpfs','/tmp','-e','HOME=/probe/home','-e','GIT_CONFIG_GLOBAL=/dev/null','-e','GIT_CONFIG_NOSYSTEM=1','-e','GIT_CONFIG_COUNT=1','-e','GIT_CONFIG_KEY_0=safe.directory','-e','GIT_CONFIG_VALUE_0=*','-e','GIT_AUTHOR_NAME=user','-e','GIT_AUTHOR_EMAIL=user@fixture.invalid','-e','GIT_COMMITTER_NAME=user','-e','GIT_COMMITTER_EMAIL=user@fixture.invalid','-e','PI_SKIP_VERSION_CHECK=1','-e','CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1','-v',f'{W}:/probe','-v',f'{repo}:/work']
    for r in repos:
        guest='/work/.git' if r==repo else '/probe/'+str(r.relative_to(W))+ '/.git'
        args+=['-v',f'{r}/.git:{guest}:ro']
        if r == repo: args+=['-v',f'{r}/.git:/probe/repo/.git:ro']
    if MODE in ('nomodel','fixture','interactive'):
        sock=W/'model.sock';server=UnixHTTP(str(sock),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
        ENV['SSH_AUTH_SOCK']=str(sock);args+=['--ssh']
    args+=['--entrypoint','sleep',IMAGE,'infinity']
    (W/'home').mkdir()
    p=run(args)
    if p.returncode: raise RuntimeError('start failed')
    started=True
    x('id; git --version; pi --version; claude --version; codex --version; grep -E " /work( |/.git )" /proc/self/mountinfo; grep CapBnd /proc/self/status')
    if MODE=='inspect':
        x('pi --help; claude --help; codex exec --help')
        x("rg -n 'APPEND_SYSTEM|SYSTEM.md|AGENTS.override|AGENTS.md|CLAUDE.md' /usr/local/lib/node_modules/@earendil-works/pi-coding-agent/dist/core/resource-loader.js /usr/local/lib/node_modules/@earendil-works/pi-coding-agent/dist/core/config* /usr/local/lib/node_modules/@earendil-works/pi-coding-agent/dist/core/system-prompt.js")
    elif MODE=='git':
        for case in ('status','add','commit','stash','checkout','fetch','gc'):
            r=W/'cases'/case
            before=snapshot(r); meta=snapshot(r,True)
            command={'status':'status','add':'add a untracked','commit':'commit -m fixture','stash':'stash push -u','checkout':'checkout other','fetch':'fetch origin','gc':'gc'}[case]
            x(f'cd /probe/cases/{case}; git {command}')
            after=snapshot(r); aftermeta=snapshot(r,True)
            emit('host delta '+json.dumps({'case':case,'worktree':[k for k in before.keys()|after.keys() if before.get(k)!=after.get(k)],'git':[k for k in meta.keys()|aftermeta.keys() if meta.get(k)!=aftermeta.get(k)]}))
            emit('host status '+git(r,'status','--porcelain'))
        # Optional index refresh fails silently during status; no lock survives.
        x('cd /probe/cases/status; strace -f -e trace=%file -o /tmp/status.trace git status --short; rg "index.lock|EROFS" /tmp/status.trace')
    elif MODE in ('nomodel','fixture','interactive'):
        (W/'bridge.js').write_text("const net=require('net');net.createServer(c=>{const s=net.connect('/var/host-services/ssh-auth.sock');c.pipe(s);s.pipe(c);c.on('error',()=>s.destroy());s.on('error',()=>c.destroy());}).listen(18080,'127.0.0.1',()=>console.log('ready'));\n")
        x('node /probe/bridge.js >/probe/bridge.out 2>&1 &')
        run(['container','exec','--user','0:0',NAME,'chmod','666','/var/host-services/ssh-auth.sock'])
        x('ls -ln /var/host-services/ssh-auth.sock')
        for _ in range(100):
            if (W/'bridge.out').exists() and 'ready' in (W/'bridge.out').read_text(): break
            threading.Event().wait(.02)
        for agent in os.environ.get('PROBE_AGENTS','pi,claude,codex').split(','):
            h=W/('home-'+agent);h.mkdir();(h/'.pi/agent').mkdir(parents=True)
            (h/'.codex').mkdir()
            (h/'.codex/AGENTS.md').write_text('CODEX_HOME_AGENTS\n'+TEXT+'\n')
            (W/'append.txt').write_text('CLAUDE_APPEND\n'+TEXT+'\n')
            (h/'.pi/agent/models.json').write_text(json.dumps({'providers':{'fixture':{'baseUrl':'http://127.0.0.1:18080/pi','api':'anthropic-messages','apiKey':'fixture-placeholder','models':[{'id':'fixture','name':'fixture','reasoning':False,'input':['text'],'contextWindow':200000,'maxTokens':4096,'cost':{'input':0,'output':0,'cacheRead':0,'cacheWrite':0}}]}}}))
            (h/'.codex/config.toml').write_text('model="gpt-5.1-codex"\nmodel_provider="fixture"\n[model_providers.fixture]\nname="fixture"\nbase_url="http://127.0.0.1:18080/codex"\nwire_api="responses"\nrequires_openai_auth=false\nrequest_max_retries=0\nstream_max_retries=0\n')
            if MODE=='interactive':
                with (h/'.codex/config.toml').open('a') as f: f.write('[projects."/work"]\ntrust_level="trusted"\n')
            prefix=f'export HOME=/probe/home-{agent} CODEX_HOME=/probe/home-{agent}/.codex PI_CODING_AGENT_DIR=/probe/home-{agent}/.pi/agent ANTHROPIC_API_KEY=fixture-placeholder ANTHROPIC_BASE_URL=http://127.0.0.1:18080/claude; '
            command={'pi':'pi --approve --no-extensions -p --provider fixture --model fixture "Run the requested command."','claude':'claude -p --dangerously-skip-permissions --model claude-sonnet-4-5 --append-system-prompt-file /probe/append.txt "Run the requested command."','codex':'codex exec --dangerously-bypass-approvals-and-sandbox --json "Run the requested command."'}[agent]
            if MODE=='interactive':
                import pty, select, fcntl, struct, termios
                command=command.replace(' -p ', ' ').replace('codex exec ', 'codex ').replace(' --json ', ' ')
                if agent=='claude':
                    (h/'.claude.json').write_text(json.dumps({'hasCompletedOnboarding':True,'lastOnboardingVersion':'2.1.325','hasAcceptedDangerousMode':True,'projects':{'/work':{'hasTrustDialogAccepted':True}}}))
                master,slave=pty.openpty()
                fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,120,0,0))
                args=['container','exec','-i','-t','-e','TERM=xterm-256color','-w','/work',NAME,'sh','-c',prefix+f'timeout --foreground -k 3 12 strace -f -s 300 -e trace=%file,process -o /probe/{agent}.trace '+command]
                emit('$ '+shlex.join(args))
                proc=sp.Popen(args,env={**ENV,'TERM':'xterm-256color'},stdin=slave,stdout=slave,stderr=slave)
                os.close(slave);chunks=[];sent=set()
                while proc.poll() is None:
                    ready,_,_=select.select([master],[],[],1)
                    if ready:
                        try:
                            chunk=os.read(master,65536);chunks.append(chunk)
                            flat=re.sub(r'\s+', '', re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', b''.join(chunks).decode(errors='replace')))
                            for marker in ('Yes,Itrustthisfolder','Yes,Iaccept'):
                                if marker in flat and marker not in sent:
                                    os.write(master,b'\x1b[B\r');sent.add(marker);emit('fixture UI accepted: '+marker)
                            if agent=='claude' and 'DoyouwanttousethisAPIkey?' in flat and 'fixture-key' not in sent:
                                os.write(master,b'\x1b[A\r');sent.add('fixture-key');emit('fixture UI accepted: fixture API key')
                            if agent=='codex' and 'Trustandcontinue' in flat and 'trust' not in sent:
                                os.write(master,b'\r');sent.add('trust');emit('fixture UI accepted: generated project')
                            if b'\x1b[6n' in chunk: os.write(master,b'\x1b[1;1R')
                        except OSError: break
                proc.wait(timeout=20);os.close(master)
                raw=b''.join(chunks).decode(errors='replace');(W/(agent+'-ui.txt')).write_text(raw)
                clean=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07)|\x1b.', '',raw)
                emit('UI tail (terminal controls removed): '+clean[-1200:]+f' [client exit {proc.returncode}; observation limited to 12s]')
            else:
                p=x(prefix+f'timeout 40 strace -f -s 300 -e trace=%file,process -o /probe/{agent}.trace '+command,timeout=60)
            lines=(W/(agent+'.trace')).read_text().splitlines()
            execs=[line for line in lines if 'execve(' in line and ('git"' in line or '/git"' in line)]
            writes=[line for line in lines if ('.git/' in line or 'index.lock' in line) and any(t in line for t in ['O_WRONLY','O_RDWR','mkdir','rename','unlink'])]
            reads=[line for line in lines if any(t in line for t in ['/AGENTS.md','/CLAUDE.md','APPEND_SYSTEM.md','SYSTEM.md']) and ('openat(' in line or 'statx(' in line)]
            emit(agent+' git exec argv (exec attempts; -c key=value pairs omitted):')
            for line in execs:
                if '= -1' in line: continue
                match=re.search(r'execve\(.*?, (\[.*\]), ',line)
                if match:
                    try:
                        argv=json.loads(match[1]);out=[];i=0
                        while i<len(argv):
                            if argv[i]=='-c': i+=2;continue
                            out.append(argv[i]);i+=1
                        emit(json.dumps(out))
                    except json.JSONDecodeError: emit(line[:400])
            emit(agent+' direct metadata reads:\n'+'\n'.join(line for line in lines if 'openat(' in line and '/work/.git/' in line and 'O_RDONLY' in line)[:1200])
            emit(agent+' metadata write attempts:\n'+'\n'.join(writes[:20]))
            emit(agent+' instruction accesses:\n'+'\n'.join(reads[:25]))
            emit(agent+' remaining Git locks: '+json.dumps([str(p.relative_to(repo)) for p in (repo/'.git').rglob('*.lock')]))
        emit('host worktree after agents: '+git(repo,'status','--porcelain'))
    elif MODE=='stale':
        for i in range(5):
            x('git status --porcelain; git log -1 --format=%s',quiet=True)
            (repo/'a').write_text(f'host {i}\n');git(repo,'add','a');git(repo,'commit','-qm',f'host-{i}')
            start=time.monotonic(); observations=[]
            while time.monotonic()-start<2.0:
                p=x('git status --porcelain; git log -1 --format=%s',quiet=True)
                t=time.monotonic()-start
                observations.append({'seconds':round(t,3),'output':p.stdout+p.stderr,'exit':p.returncode})
                if p.returncode==0 and p.stdout.strip()==f'host-{i}': break
            emit('stale trial '+str(i)+' '+json.dumps(observations))
        emit('host fsck: '+str(run(['git','-C',repo,'fsck','--full'],quiet=True)))
    elif MODE=='timing':
        emit('100000 generated files plus a; 4 guest CPUs; status includes untracked scan')
        for label,cmd,kw in [('host',['git','-C',large,'status','--porcelain'],{}),('box',['container','exec',NAME,'node','-e',"const cp=require('child_process');for(let i=0;i<4;i++){const t=performance.now();const r=cp.spawnSync('git',['-C','/probe/large','status','--porcelain'],{encoding:'utf8'});console.log(JSON.stringify({iteration:i,seconds:(performance.now()-t)/1000,exit:r.status,output:r.stdout,stderr:r.stderr}));}"],{})]:
            if label=='host':
                for i in range(4):
                    t=time.monotonic();p=run(cmd,quiet=True);emit(f'host iteration {i} seconds {time.monotonic()-t:.6f} exit {p.returncode} output {p.stdout!r}')
            else: run(cmd)
finally:
    if server: server.shutdown();server.server_close()
    if started:
        run(['container','stop',NAME]);run(['container','delete',NAME])
    emit('scratch '+str(W)+'; no image or disk created; existing image retained')
