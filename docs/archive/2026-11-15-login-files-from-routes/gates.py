#!/usr/bin/env python3
"""Manual pre-implementation gates. Synthetic fixtures only; no agent login."""
import http.server, json, os, pathlib, select, shlex, subprocess, tempfile, threading

ROOT = pathlib.Path(__file__).resolve().parents[3]
P = ROOT / 'target/debug/pinfold'
W = pathlib.Path(tempfile.mkdtemp(prefix='pf3.', dir='/tmp'))
E = {k: os.environ[k] for k in ('HOME', 'PATH', 'USER', 'LOGNAME', 'TMPDIR') if k in os.environ}
E.update(XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'), PINFOLD_NO_UPDATE_CHECK='1')
(W/'s/pinfold').mkdir(parents=True)
(W/'s/pinfold/maintenance').touch()
NAME = 'probe-pr3-gates-' + W.name[4:]
DISK = NAME + '-home'
seen = []

def emit(s):
    print(str(s).replace(str(ROOT), '/home/user/pinfold').replace(str(pathlib.Path.home()), '/home/user'), flush=True)

def run(args, check=True):
    p = subprocess.run(list(map(str, args)), env=E, text=True, capture_output=True, timeout=120)
    if check and p.returncode:
        emit(p.stdout+p.stderr)
        raise RuntimeError('command failed: '+shlex.join(list(map(str,args))))
    return p

def x(s, check=True):
    return run([P, 'box', 'exec', NAME, '--', 'sh', '-c', s], check)

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def do_POST(self):
        self.rfile.read(int(self.headers.get('Content-Length', 0)))
        seen.append((self.headers.get('Host'), self.path,
                     self.headers.get('Authorization') == 'Bearer pinfold-placeholder',
                     self.headers.get('x-api-key') == 'pinfold-placeholder'))
        self.send_response(400)
        self.send_header('Content-Length', '0')
        self.end_headers()
    def do_GET(self):
        self.send_response(404); self.send_header('Content-Length', '0'); self.end_headers()

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
files = W/'files'
for part in ('pi', 'claude', 'codex'): (files/part).mkdir(parents=True)
(files/'pi/index.ts').write_text('export default function(pi) { pi.registerProvider("anthropic", {baseUrl:"http://managed",apiKey:"pinfold-placeholder"}); }\n')
(files/'claude/managed-settings.json').write_text(json.dumps({'env':{'ANTHROPIC_BASE_URL':'http://managed','CLAUDE_CODE_OAUTH_TOKEN':'pinfold-placeholder','ANTHROPIC_API_KEY':''}}))
(files/'codex/config.toml').write_text('model_provider="pinfold"\n[model_providers.pinfold]\nname="pinfold"\nbase_url="http://managed/backend-api/codex"\nwire_api="responses"\nrequires_openai_auth=false\nrequest_max_retries=0\nstream_max_retries=0\n')
spec = {'name':NAME,'profile':'default','image':'localhost/pinfold/profile-default:latest',
        'cpus':2,'memory':'1G','env':{'HOME':'/home/user'},
        'disks':[{'name':DISK,'guest':'/home/user'}],
        'egress':{'allow':[],'routes':{n:'127.0.0.1:'+str(server.server_port) for n in ('managed','override')}}}
owner = None

def up():
    global owner
    err = (W/'up.stderr').open('w')
    owner = subprocess.Popen([str(P),'box','up'], env=E, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err, text=True)
    owner.stdin.write(json.dumps(spec)+'\n'); owner.stdin.flush()
    if not select.select([owner.stdout], [], [], 180)[0]: raise RuntimeError('readiness deadline')
    row = json.loads(owner.stdout.readline())
    if row.get('event') != 'ready':
        emit(row); emit((W/'up.stderr').read_text()); raise RuntimeError('not ready')

def down():
    global owner
    if owner:
        run([P,'box','down',NAME]); owner.stdin.close(); owner.wait(timeout=90); owner = None

def selection(label, command, expected):
    seen.clear()
    p = x('cd /tmp; timeout --foreground 35 '+command, check=False)
    emit(label+' exit='+str(p.returncode)+' requests='+json.dumps(seen))
    if not seen:
        emit((p.stdout+p.stderr)[-1800:])
        raise RuntimeError(label+': no fixture request')
    if any(row[0] != expected for row in seen): raise RuntimeError(label+': unexpected precedence')

try:
    up()
    emit(x('pi --version; claude --version; codex --version').stdout)
    x('mkdir -p /home/user/.pi/agent/extensions /home/user/project/.codex; printf user-extension > /home/user/retained')
    down()
    spec['mounts'] = [{'host':str(files/p),'guest':g} for p,g in (
        ('pi','/home/user/.pi/agent/extensions/pinfold-login'),
        ('claude','/etc/claude-code'),('codex','/etc/codex'))]
    up()
    selection('pi subdirectory autoload', 'pi -p --provider anthropic --model claude-sonnet-4-5 "Gate."', 'managed')
    x("printf '%s' '{\"anthropic\":{\"type\":\"api_key\",\"key\":\"fixture-stored\"}}' > /home/user/.pi/agent/auth.json")
    selection('pi stored credential with extension', 'pi -p --provider anthropic --model claude-sonnet-4-5 "Gate."', 'managed')
    x("printf '%s' '{\"env\":{\"ANTHROPIC_BASE_URL\":\"http://override\",\"ANTHROPIC_API_KEY\":\"fixture-user\"},\"permissions\":{\"defaultMode\":\"bypassPermissions\"},\"skipDangerousModePermissionPrompt\":true}' > /home/user/.claude/settings.json")
    selection('claude managed over user and environment', 'env ANTHROPIC_BASE_URL=http://override ANTHROPIC_API_KEY=fixture-env claude -p --model claude-sonnet-4-5 "Gate."', 'managed')
    selection('codex system config', 'codex exec --skip-git-repo-check --json "Gate."', 'managed')
    x("printf '%s\\n' 'model_provider=\"override\"' '[model_providers.override]' 'name=\"override\"' 'base_url=\"http://override\"' 'wire_api=\"responses\"' 'requires_openai_auth=false' 'request_max_retries=0' 'stream_max_retries=0' >> /home/user/.codex/config.toml")
    selection('codex user over system', 'codex exec --skip-git-repo-check --json "Gate."', 'override')
    selection('codex explicit over user', 'codex -c model_provider=\'"pinfold"\' exec --skip-git-repo-check --json "Gate."', 'managed')
    emit('gates passed; HTTP 400 establishes configuration selection only, not inference')
finally:
    down()
    run([P,'disk','rm',DISK])
    server.shutdown(); server.server_close()
    emit('gate box and home disk removed; scratch='+str(W))
