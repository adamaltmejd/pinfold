#!/usr/bin/env python3
"""Native loading only. No pinfold share exists at this HEAD. Fixture tokens only."""
import hashlib, json, os, pathlib, shutil, subprocess as sp, tempfile, uuid
D = pathlib.Path(__file__).resolve().parent
W = pathlib.Path(tempfile.mkdtemp(prefix='pfi.', dir='/tmp'))
N = 'probe-instructions-' + uuid.uuid4().hex[:8]
IMAGE = 'localhost/pinfold/profile-default'
LOG = D / 'instructions.txt'
LOG.write_text('Command: python3 docs/archive/2026-11-17-share-probes/instructions.py\n')
ENV = {**os.environ, 'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_NOSYSTEM': '1'}
def emit(s):
    s = str(s).replace(str(W), '$W').replace(str(pathlib.Path.home()), '/Users/user')
    print(s, flush=True)
    with LOG.open('a') as f: f.write(s + '\n')
def run(args, check=True, timeout=90):
    p = sp.run(list(map(str, args)), env=ENV, text=True, capture_output=True, timeout=timeout)
    if check and p.returncode: raise RuntimeError(p.stdout + p.stderr)
    return p
def x(cmd, check=True): return run(['container','exec','-w','/project',N,'sh','-c',cmd], check)
GUIDANCE = 'SHARE_GUIDANCE: Share edits reach host files; only the host writes protected Git metadata.\nDISK_GUIDANCE: Disk commits cross through pinfold fetch.\n'
server = r'''use IO::Socket::INET;
my $s=IO::Socket::INET->new(LocalAddr=>'127.0.0.1',LocalPort=>18080,Listen=>10,ReuseAddr=>1) or die $!;
open(my $ready,'>','/probe/ready');print $ready 'ready';close $ready;
while(my $c=$s->accept()) { binmode $c; my $first=<$c>; my $length=0;
while(my $line=<$c>){last if $line =~ /^\r?\n$/; $length=$1 if $line =~ /^Content-Length:\s*(\d+)/i;}
my $body=''; while(length($body)<$length){my $n=read($c,my $buf,$length-length($body));last unless $n;$body.=$buf;}
my @marks=grep {index($body,$_)>=0} qw(HOME_USER PROJECT_USER SHARE_GUIDANCE DISK_GUIDANCE);
open(my $out,'>>','/probe/requests');print $out join(',',@marks)."\n";close $out;
my $response='{"error":{"type":"invalid_request_error","message":"fixture: no model"}}';
print $c "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: ".length($response)."\r\nConnection: close\r\n\r\n".$response;close $c;
}
'''

started = False
try:
    (W/'home').mkdir(); (W/'project').mkdir(); (W/'server.pl').write_text(server)
    for a in [('init','-q','-b','main'),('config','user.name','user'),('config','user.email','user@fixture.invalid')]:
        run(['git','-C',W/'project',*a])
    (W/'project/a').write_text('base\n')
    run(['git','-C',W/'project','add','a']); run(['git','-C',W/'project','commit','-qm','base'])
    args=['container','run','-d','--name',N,'--network','none','--read-only','--cap-drop','ALL',
          '--user',f'{os.getuid()}:{os.getgid()}','--tmpfs','/tmp','-e','HOME=/home/user',
          '-e','GIT_CONFIG_GLOBAL=/dev/null','-e','GIT_CONFIG_NOSYSTEM=1',
          '-v',f'{W}:/probe','-v',f'{W}/home:/home/user','-v',f'{W}/project:/project',
          '-v',f'{W}/project/.git:/project/.git:ro','-v',f'{W}/project/.git:/probe/project/.git:ro',
          '--entrypoint','sleep',IMAGE,'infinity']
    emit('native container run: network none; read-only root; cap-drop ALL; host uid:gid; writable project + protected .git; fresh home')
    run(args); started=True
    emit(x('id; git --version; pi --version; claude --version; codex --version').stdout.strip())
    p=x('git status --porcelain; git log -1 --format=%s; git config --show-origin --get-all safe.directory',False)
    emit('Git control (no safe.directory setting): '+repr(p.stdout)+' stderr='+repr(p.stderr)+' chain_exit='+str(p.returncode))
    p=x('stat -c \"%u:%g %n\" /project /project/.git; git -c safe.directory=/project status --porcelain && git -c safe.directory=/project log -1 --format=%s',False)
    emit('Git exact-path exception control: '+repr(p.stdout)+' stderr='+repr(p.stderr)+' exit='+str(p.returncode))
    x('perl /probe/server.pl >/probe/server.out 2>&1 &')
    x('timeout 10 sh -c \"while [ ! -f /probe/ready ]; do :; done\"')
    for agent in ('pi','claude','codex'):
        for p in (W/'home').iterdir():
            shutil.rmtree(p) if p.is_dir() else p.unlink()
        for f in ('AGENTS.md','CLAUDE.md'): (W/'project'/f).write_text('PROJECT_USER: preserve project instructions.\n')
        home = W/'home'
        pi = home/'.pi/agent'; pi.mkdir(parents=True)
        codex = home/'.codex'; codex.mkdir()
        claude = home/'.claude'; claude.mkdir()
        (pi/'models.json').write_text(json.dumps({'providers':{'fixture':{'baseUrl':'http://127.0.0.1:18080/pi','api':'anthropic-messages','apiKey':'fixture-placeholder','models':[{'id':'fixture','name':'fixture','reasoning':False,'input':['text'],'contextWindow':200000,'maxTokens':4096,'cost':{'input':0,'output':0,'cacheRead':0,'cacheWrite':0}}]}}}))
        (codex/'config.toml').write_text('model="gpt-5.1-codex"\nmodel_provider="fixture"\n[model_providers.fixture]\nname="fixture"\nbase_url="http://127.0.0.1:18080/codex"\nwire_api="responses"\nrequires_openai_auth=false\nrequest_max_retries=0\nstream_max_retries=0\n')
        seed={'pi':pi/'APPEND_SYSTEM.md','claude':claude/'CLAUDE.md','codex':codex/'AGENTS.md'}[agent]
        if agent=='pi': (pi/'AGENTS.md').write_text('HOME_USER: preserve global user instruction.\n')
        command={'pi':'pi --approve --no-extensions -p --provider fixture --model fixture',
                 'claude':'claude -p --dangerously-skip-permissions --model claude-sonnet-4-5',
                 'codex':'codex exec --dangerously-bypass-approvals-and-sandbox --json'}[agent]
        prefix='export HOME=/home/user CODEX_HOME=/home/user/.codex PI_CODING_AGENT_DIR=/home/user/.pi/agent ANTHROPIC_API_KEY=fixture-placeholder ANTHROPIC_BASE_URL=http://127.0.0.1:18080/claude PI_SKIP_VERSION_CHECK=1 CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1; '
        emit(agent+' native seed /home/user/'+str(seed.relative_to(home))+'; command '+command)
        for case in ('share','disk','existing'):
            if case=='existing': seed.write_text('HOME_USER: preserve existing native instruction.\n')
            before=seed.read_bytes() if seed.exists() else None
            if not seed.exists(): seed.write_text(GUIDANCE)
            preserved=before is None or before==seed.read_bytes()
            (W/'requests').write_text('')
            p=x(prefix+'timeout --foreground -k 3 35 '+command+' "Fixture project kind '+case+'. Say hello."',False)
            rows=[line.split(',') for line in (W/'requests').read_text().splitlines()]
            marks=sorted({m for row in rows for m in row if m})
            emit(json.dumps({'agent':agent,'case':case,'exit':p.returncode,'requests':len(rows),'markers':marks,'existing_bytes_preserved':preserved,'seed_sha256':hashlib.sha256(seed.read_bytes()).hexdigest()}))
            if not rows: emit('failure output '+(p.stdout+p.stderr)[-1000:])
    emit('All HTTP responses were fixture 400; no model behavior or production seed delivery established. Share/disk cases use unchanged generic seed, new harness process and a mode prompt; no mount lifecycle switch.')
finally:
    if started:
        run(['container','stop',N],False); run(['container','delete',N],False)
    shutil.rmtree(W)
    emit('Cleanup: own container and scratch removed; no image, disk or volume created.')
