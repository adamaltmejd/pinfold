import base64,datetime,hashlib,json,os,pathlib,selectors,subprocess,time
root=pathlib.Path('/tmp/pfh.298')
def enc(x): return base64.urlsafe_b64encode(json.dumps(x,separators=(',',':')).encode()).rstrip(b'=').decode()
token=enc({'alg':'none'})+'.'+enc({'exp':int(time.time())+86400,'email':'fixture@example.invalid','https://api.openai.com/auth':{'chatgpt_account_id':'fixture-account','chatgpt_plan_type':'plus'}})+'.fixture'
requests=[{'method':'initialize','id':0,'params':{'clientInfo':{'name':'pinfold','title':'pinfold','version':'fixture'}}},{'method':'initialized'},{'method':'getAuthStatus','id':1,'params':{'includeToken':True}}]
for case in ['empty','chatgpt']:
 home=root/case
 home.mkdir(exist_ok=True)
 (home/'config.toml').write_text('cli_auth_credentials_store = "file"\n')
 if case=='chatgpt':
  (home/'auth.json').write_text(json.dumps({'auth_mode':'chatgpt','OPENAI_API_KEY':None,'tokens':{'id_token':token,'access_token':token,'refresh_token':'fixture-refresh','account_id':'fixture-account'},'last_refresh':datetime.datetime.now(datetime.timezone.utc).isoformat()}))
 env={'HOME':str(home),'CODEX_HOME':str(home),'PATH':'/usr/bin:/bin','TMPDIR':str(root),'XDG_CONFIG_HOME':str(home/'config'),'XDG_STATE_HOME':str(home/'state'),'XDG_CACHE_HOME':str(home/'cache'),'HTTPS_PROXY':'http://127.0.0.1:9','HTTP_PROXY':'http://127.0.0.1:9','ALL_PROXY':'http://127.0.0.1:9','NO_PROXY':''}
 command=['/usr/bin/sandbox-exec','-p','(version 1)(allow default)(deny network*)(deny file-read* (with errno 2) (subpath "/Users"))',str(root/'codex-app-server-aarch64-apple-darwin'),'-c','model_provider="openai"']
 print('case='+case,flush=True)
 print('command=sandbox-exec [deny all network; deny /Users reads] codex-app-server -c model_provider="openai"',flush=True)
 with (root/(case+'-stderr.txt')).open('w') as err:
  p=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,env=env,cwd=home,text=True,bufsize=1)
  for request in requests:
   print('send '+json.dumps(request),flush=True)
   p.stdin.write(json.dumps(request)+'\n')
  p.stdin.flush()
  sel=selectors.DefaultSelector();sel.register(p.stdout,selectors.EVENT_READ)
  deadline=time.monotonic()+25; answer=None
  while time.monotonic()<deadline:
   if not sel.select(max(0,deadline-time.monotonic())): break
   line=p.stdout.readline()
   if not line:break
   value=json.loads(line)
   if isinstance(value.get('params'),dict) and 'serverName' in value['params']: value['params']['serverName']='fixture-host'
   print('receive '+json.dumps(value),flush=True)
   if value.get('id')==1:
    answer=value
    break
  if answer is None: print('NO ID 1 ANSWER',flush=True)
  elif case=='chatgpt': print('matches fixture token='+str(answer.get('result',{}).get('authToken')==token),flush=True)
  p.terminate()
  try:p.wait(timeout=3)
  except subprocess.TimeoutExpired:p.kill();p.wait()
  print('helper reaped',flush=True)
