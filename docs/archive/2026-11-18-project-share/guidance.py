"""Observe native instruction delivery in actual fixture HTTP request bodies."""
import pathlib
exec(compile((pathlib.Path(__file__).with_name('apple.py')).read_text().split('try:\n p=setup')[0],__file__,'exec'))
server=r'''use IO::Socket::INET;
my $s=IO::Socket::INET->new(LocalAddr=>'127.0.0.1',LocalPort=>18080,Listen=>10,ReuseAddr=>1) or die $!;
open(my $ready,'>',"$ENV{HOME}/server-ready");close $ready;
while(my $c=$s->accept()) {my $first=<$c>;my $length=0;
while(my $line=<$c>){last if $line =~ /^\r?\n$/;$length=$1 if $line =~ /^Content-Length:\s*(\d+)/i;}
my $body='';while(length($body)<$length){my $n=read($c,my $buf,$length-length($body));last unless $n;$body.=$buf;}
open(my $out,'>>',"$ENV{HOME}/requests");print $out join(',',map {index($body,$_)>=0 ? 1 : 0} ('For a share project','For a disk project','USER_INSTRUCTION'))."\n";close $out;
my $r='{"error":{"type":"invalid_request_error","message":"fixture no model"}}';
print $c "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: ".length($r)."\r\nConnection: close\r\n\r\n".$r;close $c;}
'''
paths={'pi':'.pi/agent/APPEND_SYSTEM.md','claude':'.claude/CLAUDE.md','codex':'.codex/AGENTS.md'}
commands={'pi':'pi --approve --no-extensions -p --provider fixture --model fixture', 'claude':'claude -p --dangerously-skip-permissions --model claude-sonnet-4-5','codex':'codex exec --dangerously-bypass-approvals-and-sandbox --json'}
models={'providers':{'fixture':{'baseUrl':'http://127.0.0.1:18080/pi','api':'anthropic-messages','apiKey':'fixture-placeholder','models':[{'id':'fixture','name':'fixture','reasoning':False,'input':['text'],'contextWindow':200000,'maxTokens':4096,'cost':{'input':0,'output':0,'cacheRead':0,'cacheWrite':0}}]}}}
codex='model="gpt-5.1-codex"\nmodel_provider="fixture"\n[model_providers.fixture]\nname="fixture"\nbase_url="http://127.0.0.1:18080/codex"\nwire_api="responses"\nrequires_openai_auth=false\nrequest_max_retries=0\nstream_max_retries=0\n'
try:
 p=setup('guidance');(p/'server.pl').write_text(server);git(p,'add','server.pl');git(p,'commit','-qm','fixture-server')
 for kind in ('share','disk'):
  saved=config(p).read_text();config(p).write_text(saved.replace('"share"','"'+kind+'"'));pf(p,'start')
  ex(p,'rm -f "$HOME/server-ready"; perl server.pl > "$HOME/server.out" 2>&1 &')
  ex(p,'timeout 10 sh -c \'while [ ! -f "$HOME/server-ready" ]; do :; done\'; echo SERVER-READY')
  ex(p,'printf %s '+shlex.quote(json.dumps(models))+' > "$HOME/.pi/agent/models.json"; printf %s '+shlex.quote(codex)+' > "$HOME/.codex/config.toml"')
  for agent in paths:
   cmd='unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy ALL_PROXY all_proxy; export NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost; export ANTHROPIC_API_KEY=fixture-placeholder ANTHROPIC_BASE_URL=http://127.0.0.1:18080/claude PI_SKIP_VERSION_CHECK=1 CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1; : > "$HOME/requests"; timeout --foreground -k 3 35 '+commands[agent]+' "Fixture hello" > "$HOME/agent.out" 2>&1; result=$?; echo AGENT-EXIT=$result; cat "$HOME/requests"; if [ ! -s "$HOME/requests" ]; then tail -c 1600 "$HOME/agent.out"; fi'
   r=ex(p,cmd);rows=[x for x in r.stdout.decode().splitlines() if x.count(',')==2];assert rows,(agent,r.stdout)
   expected='1,1,0' if kind=='share' else '0,0,1';assert expected in rows,(agent,kind,rows);say(f'MODEL-REQUEST agent={agent} kind={kind} share/disk/user={expected}; HTTP=400 no compliance claim')
   if kind=='share':ex(p,'printf USER_INSTRUCTION > "$HOME/'+paths[agent]+'"')
  pf(p,'stop')
 say('PASS native seeds delivered; share-to-disk preserves existing user native files, guidance absent as specified')
finally:
 for p in projects:pf(p,'stop')
 for row in pf(W,'disk','list','--json').stdout.splitlines():pf(W,'disk','rm',json.loads(row)['name'],'--json')
 assert not pf(W,'disk','list','--json').stdout.strip();shutil.rmtree(W);say('guidance fixture-removed='+str(not W.exists()))
