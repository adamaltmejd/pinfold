"""Manual real-binary probe. Usage: probe.py /path/to/pinfold [existing-image] [--refresh-only]."""
import base64, collections, datetime, hashlib, http.server, json, os, pathlib
import select, shutil, subprocess, sys, tempfile, threading, time, uuid
import guest

arguments = [arg for arg in sys.argv[1:] if arg != '--refresh-only']
refresh_only = '--refresh-only' in sys.argv[1:]
BIN = pathlib.Path(arguments[0]).resolve()
IMAGE = arguments[1] if len(arguments) > 1 else 'localhost/pinfold/profile-probe-agents:latest'
W = pathlib.Path(tempfile.mkdtemp(prefix='pfc.', dir='/tmp')).resolve()
original_home = str(pathlib.Path.home())
cache = os.environ.get('XDG_CACHE_HOME', original_home + '/.cache')
ENV = {k: os.environ[k] for k in ('PATH', 'TMPDIR') if k in os.environ}
ENV.update(HOME=original_home, USER='user', LOGNAME='user', XDG_CACHE_HOME=cache,
           XDG_CONFIG_HOME=str(W/'config'), XDG_STATE_HOME=str(W/'state'),
           CODEX_HOME=str(W/'auth'), PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
for directory in ['auth', 'state/pinfold']:
    (W/directory).mkdir(parents=True)
(W/'state/pinfold/maintenance').touch()
(W/'auth/config.toml').write_text('cli_auth_credentials_store = "file"\n')
suffix = uuid.uuid4().hex[:8]
static = 'HOSTONLY-static-' + suffix
account = 'HOSTONLY-account-' + suffix

def enc(value):
    return base64.urlsafe_b64encode(json.dumps(value).encode()).rstrip(b'=').decode()

def token(expiry, serial=0):
    return enc({'alg':'none'}) + '.' + enc({'exp':expiry, 'fixture_serial':serial, 'email':'fixture@example.invalid',
        'https://api.openai.com/auth':{'chatgpt_account_id':account, 'chatgpt_plan_type':'plus'}}) + '.fixture'

jwt = token(int(time.time()) + 86400)
def store(value):
    (W/'auth/auth.json').write_text(json.dumps({'auth_mode':'chatgpt','OPENAI_API_KEY':None,
        'tokens':{'id_token':value,'access_token':value,'refresh_token':'HOSTONLY-refresh-'+suffix,
                  'account_id':account},
        'last_refresh':datetime.datetime.now(datetime.timezone.utc).isoformat()}))
store(jwt)
ENV['FIXTURE_HOST_TOKEN'] = static
markers = [static.encode(), account.encode(), jwt.encode(), ('HOSTONLY-refresh-'+suffix).encode()]
failures, captures, owners, disks, diagnostics, oauth = [], [], [], [], [], []
oauth_expected_refresh = 'HOSTONLY-refresh-' + suffix
oauth_lock = threading.Lock()
FIELDS = {'authorization', 'x-api-key', 'api-key', 'x-goog-api-key', 'x-auth-token',
          'x-access-token', 'chatgpt-account-id', 'openai-organization', 'openai-project', 'cookie', 'cookie2'}
BODY = b'fixture-body\x00\xff?token=body-is-not-inspected'


def scrub(text):
    for marker in markers:
        text = text.replace(marker.decode(), '[HOST-MARKER]')
    return text.replace(original_home, '/Users/user').replace(str(W), '$W')

def emit(text):
    print(scrub(str(text)), flush=True)

def check(condition, label):
    if not condition:
        failures.append(label)
        emit('FAIL ' + label)

def run(args, source=None, timeout=120, required=True):
    p = subprocess.run([str(BIN), *args], input=source, text=True, env=ENV, cwd=W,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, errors='replace')
    diagnostics.extend([p.stdout.encode(), p.stderr.encode()])
    if required and p.returncode:
        raise RuntimeError(scrub('command failed: ' + ' '.join(args) + '\n' + p.stdout + p.stderr))
    return p

class Fixture(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def do_POST(self):
        global oauth_expected_refresh
        headers = list(self.headers.raw_items())
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        if self.path == '/oauth/token':
            data = json.loads(body)
            with oauth_lock:
                serial = len(oauth)+1
                fresh = token(int(time.time()) + 15, serial)
                refresh = 'HOSTONLY-refreshed-' + suffix + '-' + str(serial)
                markers.extend([fresh.encode(), refresh.encode()])
                oauth.append({'matched':data.get('refresh_token') == oauth_expected_refresh,
                              'grant':data.get('grant_type'), 'expiry':time.monotonic()+16, 'token':fresh})
                oauth_expected_refresh = refresh
            response = json.dumps({'id_token':fresh,'access_token':fresh,'refresh_token':refresh}).encode()
            self.send_response(200); self.send_header('Content-Length',str(len(response)))
            self.end_headers(); self.wfile.write(response)
            return
        row = {'id':self.headers.get('X-Probe-Id'), 'path':self.path,
               'headers':headers, 'body':body}
        response = b'fixture-ok'
        if self.path.endswith('/reflect'):
            response = json.dumps([(n,v) for n,v in headers if n.lower() in FIELDS]).encode()
            row['reflected'] = hashlib.sha256(response).hexdigest()
        captures.append(row)
        self.send_response(200)
        self.send_header('Content-Length', str(len(response)))
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(response)
    def log_message(self, *args):
        pass

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
threading.Thread(target=server.serve_forever, daemon=True).start()
origin = 'http://127.0.0.1:' + str(server.server_port)
ENV['CODEX_REFRESH_TOKEN_URL_OVERRIDE'] = origin + '/oauth/token'

def login(kind, provider=None):
    value = {'login':kind, 'to':origin}
    if provider:
        value['provider'] = provider
    if kind != 'codex' and provider != 'openai-codex':
        value['from'] = 'FIXTURE_HOST_TOKEN'
    return value

routes = {'claude':login('claude'), 'codex':login('codex'),
          'pi-codex':login('pi','openai-codex'), 'pi-openai':login('pi','openai'),
          'pi-router':login('pi','openrouter'), 'pi-anthropic':login('pi','anthropic')}
ordinary = {'inject':{'to':origin,'headers':{'Authorization':{'from':'FIXTURE_HOST_TOKEN','prefix':'Bearer '}}},
            'service':'127.0.0.1:' + str(server.server_port)}
expected = {name:[('authorization', 'Bearer ' + (jwt if name in ['codex','pi-codex'] else static))]
            for name in [*routes,'pi-subscription']}
expected['pi-anthropic'] = [('x-api-key', static)]
for name in ['codex','pi-codex']:
    expected[name].append(('chatgpt-account-id', account))
expected['inject'] = [('authorization', 'Bearer ' + static)]


def up(label, route_map, harnesses):
    name = 'probe-credentials-' + label + '-' + suffix
    disk = name + '-home'
    spec = {'name':name,'image':IMAGE,'harnesses':harnesses,'cpus':2,'memory':'1G',
            'env':{'HOME':'/home/user'},'disks':[{'name':disk,'guest':'/home/user'}],
            'egress':{'routes':route_map}}
    err = tempfile.TemporaryFile(mode='w+t')
    p = subprocess.Popen([str(BIN),'box','up'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,
                         stderr=err,text=True,env=ENV,cwd=W)
    owners.append((name,p,err))
    disks.append(disk)
    p.stdin.write(json.dumps(spec)+'\n'); p.stdin.flush()
    if not select.select([p.stdout],[],[],180)[0]:
        raise RuntimeError('readiness timeout: ' + label)
    line = p.stdout.readline()
    diagnostics.append(line.encode())
    obj = json.loads(line)
    if obj.get('event') != 'ready':
        raise RuntimeError(scrub('up refused: ' + line))
    emit('ready ' + label + '; image=' + obj['image']['ref'])
    return name, obj


def down(name):
    run(['box','down',name],timeout=90)
    for owner,p,err in owners:
        if owner == name and p.poll() is None:
            p.stdin.close(); p.wait(timeout=90)
            diagnostics.append(p.stdout.read().encode())
            err.seek(0); diagnostics.append(err.read().encode())


def observe(label, route_map, harnesses):
    initial_failures = len(failures)
    name, obj = up(label, route_map, harnesses)
    first = len(captures)
    policies = {route:('login' if 'login' in value else 'ordinary')
                for route,value in route_map.items()}
    p = run(['box','exec',name,'--','sh','-s'], guest.script(policies), timeout=240)
    results = [json.loads(line) for line in p.stdout.splitlines()]
    rows = captures[first:]
    by_id = {row['id']:row for row in rows}
    events = [json.loads(line) for line in pathlib.Path(obj['egress_log']).read_text().splitlines()]
    requests = [result for result in results if result[1] != 'reflection-digest']
    check(len(events) == len(requests), label + ' one audit decision per request')
    counts = collections.Counter()
    refusal_upstream = collections.Counter()
    for index,(route,case,status) in enumerate(requests):
        refused = case.startswith('query-') or case.startswith('bad-') and not case.endswith('-corrected') or case == 'nominated-length'
        code = 403 if case.startswith('query-') else 400 if refused else 200
        reason = 'login credential query' if code == 403 else 'ambiguous framing' if case == 'nominated-length' else 'malformed request'
        check(status == code, route+'/'+case+' status='+str(status)+' expected='+str(code))
        captured = by_id.get(route+'/'+case)
        check((captured is None) == refused, route+'/'+case+' upstream admission')
        if refused and captured is not None:
            refusal_upstream[route] += 1
        if refused and index < len(events):
            check(events[index].get('reason') == reason, route+'/'+case+' audit reason')
        counts[(route,str(code))] += 1
    for route in route_map:
        row = by_id[route+'/hardest']
        headers = [(n.lower(),v) for n,v in row['headers']]
        check(row['body'] == BODY, route+' exact binary body')
        check(row['path'] == '/arbitrary/non-model/hardest', route+' arbitrary path')
        for field, values in [('anthropic-version',['2023-06-01']),('anthropic-beta',['beta-one, beta-two']),
                              ('x-trace-keep',['keep-first','keep-second'])]:
            check([v for n,v in headers if n == field] == values, route+' ordered '+field)
        if policies[route] == 'login':
            auth = [(n,v) for n,v in headers if n in FIELDS or n.startswith('proxy-')]
            unnominated = [(n.lower(),v) for n,v in by_id[route+'/unnominated']['headers']
                           if n.lower() in FIELDS or n.lower().startswith('proxy-')]
            check(unnominated == expected[route], route+' credential filtering without nominations')
            check(auth == expected[route], route+' only ordered trusted credential fields')
            check(not any(n == 'x-trace-drop' for n,v in headers), route+' Connection drops metadata')
            check(headers[0] == ('host', origin.removeprefix('http://')), route+' regenerated Host')
            check(headers[-len(auth)-1:-1] == auth, route+' trusted fields appended after guest')
            check(by_id[route+'/harmless']['path'].endswith('?safe=%61%2f%3B&plus=a+b;empty=&bare&tokenized=yes'), route+' exact query bytes')
            beta = [(n.lower(),v) for n,v in by_id[route+'/nominated-beta']['headers']]
            check(not any(n == 'anthropic-beta' for n,v in beta) and ('anthropic-version','2023-06-01') in beta, route+' nominated beta only')
            digest = next(value for r,c,value in results if r == route and c == 'reflection-digest')
            check(digest == by_id[route+'/reflect']['reflected'], route+' reflected credential response reached guest')
        else:
            row = by_id[route+'/ordinary-query']
            ordinary_headers = [(n.lower(),v) for n,v in row['headers']]
            check(row['path'].endswith('?token=&api_key&safe=%GG') and row['body'] == BODY, route+' unchanged ordinary query/body')
            check(('cookie','guest-cookie') in ordinary_headers and ('x-api-key','guest-key') in ordinary_headers and
                  ('x-trace-drop','retain-me') in ordinary_headers, route+' ordinary credential/nominations retained')
            check([v for n,v in ordinary_headers if n == 'authorization'] ==
                  (['Bearer '+static] if route == 'inject' else ['guest-auth']), route+' ordinary Authorization behavior')
        summary = ', '.join(code+'='+str(counts[(route,code)]) for code in ['200','400','403'])
        observed = collections.Counter(n for n,v in headers if n in FIELDS or n.startswith('proxy-'))
        trusted = ','.join(n+':'+str(count) for n,count in observed.items())
        outcome = 'passed' if len(failures) == initial_failures else 'failed'
        emit(route + ': ' + summary + '; observed_credential_counts='+trusted+'; refused_upstream_requests='+str(refusal_upstream[route])+'; checks='+outcome)
    scan_guest(name, label, harnesses)
    scan_state()
    outcome = 'passed' if len(failures) == initial_failures else 'failed'
    emit(label + ': guest/harness environment, home, generated files, live state, reflection checks='+outcome)
    emit(label + ': fixture_requests='+str(len(rows))+'; audit_decisions='+str(len(events)))
    down(name)

def scan_state():
    for path in (W/'state').rglob('*'):
        if not path.is_file() or path.is_symlink() or path.suffix in ['.ext4','.raw','.img','.qcow2']:
            continue
        with path.open('rb') as stream:
            overlap = b''
            while chunk := stream.read(65536):
                check(not any(marker in overlap+chunk for marker in markers), 'state/audit host-marker scan')
                overlap = chunk[-2048:]


def scan_guest(name, label, harnesses):
    source = "env; find /home/user /etc/codex /opt/pinfold/login -type f -exec cat {} + 2>/dev/null || true\n"
    for harness in [None, *harnesses]:
        args = ['box','exec',name] + (['--harness',harness] if harness else [])
        data = run([*args,'--','sh','-s'], 'env\n' if harness else source).stdout.encode()
        check(not any(marker in data for marker in markers),
              label+' '+(harness or 'base')+' guest environment/home/generated-file marker scan')


def single(name, route, case, query='', connection=None):
    headers = [('Connection',connection)] if connection else []
    source = 'set -eu\n' + guest.command(route,case,query,headers)
    results = run(['box','exec',name,'--','sh','-s'],source).stdout.splitlines()
    return json.loads(results[-1])[2]


def refresh():
    initial_failures = len(failures)
    near = token(int(time.time())+365)
    markers.append(near.encode()); store(near)
    name,obj = up('refresh', {key:routes[key] for key in ['codex','pi-codex']}, ['codex','pi'])
    first = len(captures)
    for route in ['codex','pi-codex']:
        check(single(name,route,'before-refresh') == 200, route+' before refresh control')
    check(len(oauth) == 0, 'no proactive OAuth before refresh window')
    # Token expiry and the production 60-second ask interval use wall time.
    deadline = time.monotonic()+66
    while time.monotonic() < deadline:
        time.sleep(max(0,min(1,deadline-time.monotonic())))
    admitted = len(captures)
    check(single(name,'codex','pre-refresh-query','?token=guest') == 403, 'query validation before refresh')
    check(single(name,'pi-codex','pre-refresh-connection',connection='not a token') == 400, 'Connection validation before refresh')
    check(len(oauth) == 0 and len(captures) == admitted, 'pre-refresh refusals never refresh or forward')
    pre_events = [json.loads(line) for line in pathlib.Path(obj['egress_log']).read_text().splitlines()]
    check([event.get('reason') for event in pre_events[-2:]] == ['login credential query','malformed request'], 'pre-refresh refusal audit reasons')
    check(single(name,'codex','after-refresh') == 200, 'codex after refresh control')
    refresh_count = len(oauth)
    check(refresh_count >= 1, 'real helper reached local OAuth fixture')
    grants_match = all(row['matched'] and row['grant'] == 'refresh_token' for row in oauth)
    check(grants_match, 'every OAuth grant follows synthetic refresh-token rotation')
    emit('refresh fixture: oauth_posts='+str(refresh_count)+'; grant_checks='+
         json.dumps([{'refresh_matches':row['matched'],'grant_matches':row['grant']=='refresh_token'} for row in oauth[:8]],separators=(',',':'))+'; all_grants_match='+str(grants_match))
    fresh = oauth[-1]['token'] if oauth else ''
    check(single(name,'pi-codex','after-refresh') == 200, 'pi-codex after refresh control')
    check(len(oauth) == refresh_count, 'Pi reuses shared refreshed token without OAuth')
    for row in captures[first:]:
        wanted = near if row['id'].endswith('/before-refresh') else fresh
        auth = [(n.lower(),v) for n,v in row['headers'] if n.lower() in FIELDS]
        check(auth == [('authorization','Bearer '+wanted),('chatgpt-account-id',account)],
              row['id']+' exact shared token/account')
    if oauth:
        while time.monotonic() < oauth[-1]['expiry']:
            time.sleep(max(0,min(1,oauth[-1]['expiry']-time.monotonic())))
    admitted = len(captures)
    for route in ['codex','pi-codex']:
        check(single(name,route,'expired') == 403, route+' expired refusal')
        check(single(name,route,'expired-query','?token=guest') == 403, route+' query before expired-token check')
    check(len(captures) == admitted and len(oauth) == refresh_count, 'expired/query refusals never reach fixture or refresh')
    events = [json.loads(line) for line in pathlib.Path(obj['egress_log']).read_text().splitlines()]
    reasons = [event.get('reason') for event in events[-4:]]
    check(reasons == ['login lapsed','login credential query']*2, 'expired/query refusal audit reasons')
    scan_guest(name, 'refresh', ['codex','pi']); scan_state(); down(name)
    outcome = 'passed' if len(failures) == initial_failures else 'failed'
    emit('refresh: checks='+outcome+'; oauth_posts='+str(refresh_count)+'; shared refreshed token controls=4; expired refusals=2; query-before-token refusals=2; pre-refresh query/Connection refusals=2')
    emit('refresh: base/harness environment, home, generated files, live state/audit marker checks='+outcome+'; synthetic auth store excluded')
    store(jwt)

start = time.monotonic()
emit('command: probe.py PINFOLD EXISTING_IMAGE; mode='+('refresh-only' if refresh_only else 'full')+'; scratch=$W; image='+IMAGE)
try:
    emit('binary=' + run(['--version']).stdout.strip())
    pins = json.loads(run(['artifacts']).stdout)
    emit('pins='+json.dumps([{k:pin[k] for k in ['name','version','cached']} for pin in pins],separators=(',',':')))
    if not refresh_only:
        observe('shared', {**routes,**ordinary}, ['claude','codex','pi'])
        observe('subscription', {'pi-subscription':login('pi','anthropic-subscription')}, ['pi'])
    refresh()
finally:
    for name,p,err in owners:
        try:
            if p.poll() is None:
                down(name)
        except Exception as error:
            check(False, 'box cleanup: ' + scrub(str(error)))
    for disk in disks:
        p = run(['disk','rm',disk,'--json'], required=False)
        check(p.returncode == 0, 'disk cleanup '+disk)
    server.shutdown(); server.server_close()
    for data in diagnostics:
        check(not any(marker in data for marker in markers), 'diagnostic host-marker scan')
    scan_state()
    emit('cleanup: own boxes stopped; own disks removed; no image built or removed')
    emit('elapsed_seconds=%.3f; failures=%d' % (time.monotonic()-start,len(failures)))
    shutil.rmtree(W)
if failures:
    sys.exit(1)
