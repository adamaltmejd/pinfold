#!/usr/bin/env python3
"""Manual real-binary acceptance: comparison.py BIN IMAGE [DISTINCT_IMAGE2]."""
import copy, hashlib, http.server, json, os, pathlib, re, subprocess as sp, sys, tempfile, threading, uuid
B = str(pathlib.Path(sys.argv[1]).resolve()); IMAGE = sys.argv[2]; IMAGE2 = sys.argv[3] if len(sys.argv) > 3 else None
W = pathlib.Path(tempfile.mkdtemp(prefix='pfoc.', dir='/tmp')).resolve(); P = W/'p'; P.mkdir(); (P/'sub').mkdir()
E = {k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
E.update(XDG_CACHE_HOME=os.environ.get('XDG_CACHE_HOME',str(pathlib.Path.home()/'.cache')), XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'), CODEX_HOME=str(W/'codex'), PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
S = W/'s/pinfold'; S.mkdir(parents=True); (S/'maintenance').touch()
VALUE = 'fixture-only-' + uuid.uuid4().hex; VALUE2 = 'fixture-other-' + uuid.uuid4().hex
E.update(PFO_VALUE=VALUE, PFO_VALUE2=VALUE2, PFO_OTHER=VALUE, PINFOLD_ENV_COMPARE=os.fsdecode(b'fixture-bytes-\xff\n'))
receipts = []
class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        valid = self.path == '/receipt' or self.headers.get('Authorization') == 'Bearer ' + VALUE
        if self.path == '/inject': valid &= self.headers.get('X-Optional') == VALUE2
        receipts.append((self.path, valid)); self.send_response(200 if valid else 401); self.end_headers(); self.wfile.write(b'RECEIPT')
    def log_message(self, *_): pass
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture); thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
target = 'http://127.0.0.1:' + str(server.server_port)
def git(*a): return sp.run(['git','-C',str(P),*a], check=True, capture_output=True)
def commit(): git('add','.'); git('-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','-qm','fixture')
git('init','-q'); (P/'tracked').write_text('HOST-ONE'); (P/'sub/tracked').write_text('sub'); commit()
ID = 'p-' + hashlib.sha256(os.fsencode(P)).hexdigest()[:12]; C = W/'c/pinfold/projects'/ID/'pinfold.toml'; C.parent.mkdir(parents=True)
PROFILE = W/'c/pinfold/profiles/default'; PROFILE.mkdir(parents=True)
for leaf in ('home','share'): (PROFILE/leaf).mkdir()
(PROFILE/'pinfold.toml').write_text(''); (PROFILE/'Containerfile').write_text('FROM fixture-recipe-one\n')
(PROFILE/'home/seed-fixture').write_text('SEED-ONE'); (PROFILE/'share/shared-fixture').write_text('SHARE-ONE')
HOST = pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'; L = HOST/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project')
ALIAS = f'localhost/pinfold/image-probe-owner-{os.getpid()}-{W.name.rsplit('.',1)[-1]}:latest' if sys.platform == 'darwin' and IMAGE2 else None
def native_tag(source):
    r = sp.run(['container','image','tag',source,ALIAS], capture_output=True, timeout=30)
    assert r.returncode == 0, 'native probe alias publication failed'
def invoke(args, env=None, cwd=P):
    stamp = pathlib.Path((env or E)['XDG_STATE_HOME'])/'pinfold/maintenance'; stamp.parent.mkdir(parents=True,exist_ok=True); stamp.touch()
    r = sp.run([B,*args], cwd=cwd, env=env or E, input=b'', capture_output=True, timeout=100)
    assert VALUE.encode() not in r.stderr and VALUE2.encode() not in r.stderr, 'credential in diagnostics'
    return r
def checked(args, expected=b'', env=None, cwd=P):
    r = invoke(args, env, cwd); assert r.returncode == 0 and r.stdout == expected, 'working control failed'; return r
baseline = dict(mounts=[dict(host='.',kind='disk')], profile='default', image=ALIAS or IMAGE, allow=['api.github.com','registry.npmjs.org'], routes={
    'receipt':f'127.0.0.1:{server.server_port}',
    'injected':dict(to=target, headers={'Authorization':dict(**{'from':'PFO_VALUE'},prefix='Bearer '),'X-Optional':dict(**{'from':'PFO_VALUE2'},prefix='')}),
    'login':dict(login='pi',provider='openrouter',**{'from':'PFO_VALUE'},to=target)})
# JSON objects are valid inline TOML tables only after their key separators change.
def config(data):
    def toml(v):
        if isinstance(v, dict): return '{'+', '.join(json.dumps(k)+' = '+toml(x) for k,x in v.items())+'}'
        if isinstance(v, list): return '['+', '.join(map(toml,v))+']'
        return json.dumps(v)
    C.write_text('\n'.join(k+' = '+toml(v) for k,v in data.items())+'\n')
old = None
def same(): assert json.loads(L.read_text())['generation'] == old, 'generation changed'
def restore(): config(baseline); checked(['--','sh','-c','printf CONTROL'],b'CONTROL'); same()
def mismatch(label, expected, data=None, env=None, restored=True):
    config(data or baseline); r = invoke(['--','sh','-c','printf MUST-NOT-RUN'], env)
    assert r.returncode == 125 and not r.stdout and b'config-changed:' in r.stderr, label+' did not refuse before CMD'
    fields = r.stderr.rsplit(b'config-changed:',1)[1].strip().split(b', ')
    categories = {'roots.config','roots.state','roots.cache','roots.home','roots.codex_home','repository','profile','image.source','image.ref','image.id','resources','mounts','disks','allow','routes.declarations','routes.values','env.names','env.values'}
    assert expected.encode() in fields and fields == sorted(fields) and all(f.decode() in categories for f in fields), label+' invalid/missing/unsorted categories'
    assert not re.search(rb'\b[0-9a-f]{64}\b', r.stderr), label+' exposed a fingerprint'
    same()
    if restored: restore()
    print(label, 'exit=125 category='+expected+' marker=absent restore='+('same-generation' if restored else 'pending'), flush=True)
def mutate(label, expected, change):
    data = copy.deepcopy(baseline); change(data); mismatch(label, expected, data)
try:
    if ALIAS: native_tag(IMAGE)
    config(baseline); checked(['start']); old = json.loads(L.read_text())['generation']; checked(['start']); same()
    checked(['--','sh','-c','test "$(cat "$HOME/seed-fixture")" = SEED-ONE; test "$(cat tracked)" = HOST-ONE'])
    for name, value in [('PINFOLD_CPUS','1'),('PINFOLD_MEMORY','512M')]: mismatch(name,'resources',env=dict(E,**{name:value}))
    mutate('allow-order','allow',lambda d:d['allow'].reverse())
    mutate('profile-name','profile',lambda d:d.update(profile='documents'))
    PROFILE.rename(W/'saved-profile')
    try: mismatch('profile-user-to-builtin','profile',restored=False)
    finally: (W/'saved-profile').rename(PROFILE)
    restore(); print('profile-user-to-builtin restore=same-generation',flush=True)
    mutate('image-reference','image.ref',lambda d:d.update(image=IMAGE if ALIAS else IMAGE+'-absent-fixture'))
    mutate('image-source','image.source',lambda d:d.pop('image'))
    if ALIAS:
        native_tag(IMAGE2)
        try:
            mismatch('image-id-same-reference','image.id',restored=False)
        finally: native_tag(IMAGE)
        restore(); print('image-id-same-reference restore=same-generation',flush=True)
    elif IMAGE2: mutate('image-id-plus-ref','image.id',lambda d:d.update(image=IMAGE2))
    else: print('NOT-RUN image.id: requires distinct prepared IMAGE2',flush=True)
    mutate('route-name','routes.declarations',lambda d:d['routes'].update(renamed=d['routes'].pop('receipt')))
    mutate('route-kind','routes.declarations',lambda d:d['routes'].update(receipt=dict(to=target,headers={})))
    mutate('route-provider','routes.declarations',lambda d:d['routes']['login'].update(provider='openai'))
    mutate('route-target','routes.declarations',lambda d:d['routes']['injected'].update(to='http://127.0.0.1:'+str(server.server_port+1 if server.server_port<65535 else server.server_port-1)))
    mutate('route-optional-target','routes.declarations',lambda d:d['routes']['login'].pop('to'))
    mutate('route-header','routes.declarations',lambda d:d['routes']['injected']['headers'].update(Renamed=d['routes']['injected']['headers'].pop('Authorization')))
    mutate('route-prefix','routes.declarations',lambda d:d['routes']['injected']['headers']['Authorization'].update(prefix='Token '))
    mutate('route-optional-prefix','routes.declarations',lambda d:d['routes']['injected']['headers']['X-Optional'].pop('prefix'))
    mutate('route-source','routes.declarations',lambda d:d['routes']['injected']['headers']['Authorization'].update(**{'from':'PFO_OTHER'}))
    for label, value in [('changed','fixture-changed'),('empty',''),('absent',None)]:
        e = dict(E)
        if value is None: e.pop('PFO_VALUE')
        else: e['PFO_VALUE'] = value
        mismatch('route-value-'+label,'routes.values',env=e)
    for label, value in [('bytes',os.fsdecode(b'changed-\xfe\n')),('empty',''),('absent',None)]:
        e = dict(E)
        if value is None: e.pop('PINFOLD_ENV_COMPARE')
        else: e['PINFOLD_ENV_COMPARE'] = value
        mismatch('forwarded-'+label,'env.names' if value is None else 'env.values',env=e)
    mismatch('forwarded-added','env.names',env=dict(E,PINFOLD_ENV_EXTRA=''))
    for label, name in [('config','XDG_CONFIG_HOME'),('state','XDG_STATE_HOME'),('cache','XDG_CACHE_HOME'),('home','HOME'),('codex_home','CODEX_HOME')]:
        mismatch('roots-'+label,'roots.'+label,env=dict(E,**{name:str(W/('other-'+label))}))
    for route in ('receipt','injected','login'): checked(['--','curl','-fsS','http://'+route+'/'+('inject' if route=='injected' else route)],b'RECEIPT')
    denied = checked(['--','curl','-sS','-o','/dev/null','-w','%{http_code}','http://example.com'],b'403')
    decisions = [json.loads(line) for line in (S/'runs'/old/'egress.jsonl').read_text().splitlines()]
    assert any(d.get('reason') == 'not allowlisted' for d in decisions) and len(receipts) == 3 and all(ok for _,ok in receipts)
    print('original-policy deny=403/not-allowlisted allowed-fixture=receipt static-credentials=checked',flush=True)
    reordered = copy.deepcopy(baseline); reordered['routes'] = dict(reversed(list(reordered['routes'].items()))); config(reordered); checked(['--','true']); same(); restore()
    (P/'tracked').write_text('HOST-TWO'); commit(); checked(['--','sh','-c','rm -f "$HOME/seed-fixture"'])
    (PROFILE/'Containerfile').write_text('FROM fixture-recipe-two\n'); (PROFILE/'home/seed-fixture').write_text('SEED-TWO'); (PROFILE/'share/shared-fixture').write_text('SHARE-TWO')
    checked(['--','sh','-c','test "$(cat tracked)" = HOST-ONE; test ! -e "$HOME/seed-fixture"; printf EXCLUDED'],b'EXCLUDED')
    checked(['--','pwd'],os.fsencode(P/'sub')+b'\n',env=dict(E,TERM='fixture-term',COLORTERM='fixture-color'),cwd=P/'sub'); same()
    checked(['--','sh','-c','printf "%s\\n" "$@"','argv','','a b',';$x'],b'\na b\n;$x\n')
    if sys.platform == 'linux': checked(['--','cat','/opt/pinfold/profile/shared-fixture'],b'SHARE-ONE')
    print('exclusions argv/cwd/TERM recipe/seed/share join=same-generation no-Git-sync missing-seed=not-reseeded',flush=True)
    values = [VALUE.encode(),VALUE2.encode(),os.fsencode(E['PINFOLD_ENV_COMPARE'])]; values += [v.hex().encode() for v in values]; scanned = 0
    for path in S.rglob('*'):
        if path.is_file() and path.stat().st_size <= 16*1024*1024:
            data = path.read_bytes(); assert all(value not in data for value in values), 'credential in retained metadata'; scanned += 1
            if path.suffix in ('.json','.jsonl'): assert b'"snapshot"' not in data and b'"fingerprint"' not in data, 'comparison metadata retained'
    print('metadata-scan raw/hex-credentials=absent snapshot/fingerprint-keys=absent files='+str(scanned)+' limit=16MiB-per-file opaque-disks=not-scanned',flush=True)
finally:
    config(baseline)
    cleanup_ok = True
    for args in (['stop'],['disk','rm','--all']):
        try:
            r = invoke(args); cleanup_ok &= r.returncode == 0; print('cleanup '+args[0]+' exit='+str(r.returncode),flush=True)
        except Exception: cleanup_ok = False; print('cleanup '+args[0]+' failed',flush=True)
    if ALIAS:
        try: native_tag(IMAGE); print('retained-image-alias='+ALIAS,flush=True)
        except Exception: cleanup_ok = False; print('probe alias restoration failed',flush=True)
    server.shutdown(); server.server_close(); thread.join(); print('fixture=$W',flush=True)
    assert cleanup_ok, 'fixture cleanup failed'
