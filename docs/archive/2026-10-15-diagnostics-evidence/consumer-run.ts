import {BoxOwner, observeList} from './consumer.ts';
import {mkdirSync,writeFileSync} from 'node:fs';
const root='/private/tmp/pd-1015/consumer';
for(const [key,suffix] of [['XDG_STATE_HOME','s'],['XDG_CONFIG_HOME','c'],['XDG_CACHE_HOME','k']]) {process.env[key!]=`${root}/${suffix}`;mkdirSync(`${root}/${suffix}/pinfold`,{recursive:true});}
writeFileSync(`${root}/s/pinfold/maintenance`,'');
const binary='/private/tmp/pd-1015/pinfold-baseline';
const spec={image:'pinfold/profile-default:latest',env:{HOME:'/tmp'},cpus:1,memory:'512M'};
const evidence:Record<string,unknown>={};
const box=new BoxOwner(binary,'pd-consumer',spec);
try {
 evidence.ready=await box.ready();
 let stdout=0,stderr=0;
 evidence.largeOutput=await box.exec(['/bin/sh','-c','head -c 200000 /dev/zero; printf "err" >&2; exit 3'],(stream,bytes)=>{if(stream==='stdout')stdout+=bytes.length;else stderr+=bytes.length;});
 const result=evidence.largeOutput as any;
 evidence.largeOutput={code:result.code,stdoutBytesDelivered:stdout,stderrBytesDelivered:stderr,retained:result.stdout.text.length,truncated:result.stdout.truncated,stderr:result.stderr};
 const cancel=new AbortController(); cancel.abort();
 evidence.cancelObserver=await observeList(binary,cancel.signal);
 evidence.afterObserver=await box.exec(['/bin/sh','-c','printf still-alive']);
 await box.stop();evidence.complete=box.snapshot();
 const missing=new BoxOwner(binary,'pd-consumer-missing',{...spec,image:'pinfold/diag-does-not-exist:absent'});
 try {await missing.ready();}catch(error){evidence.refused={lifecycle:(error as any).lifecycle,evidence:(error as any).evidence};}
 const killed=new BoxOwner(binary,'pd-consumer-killed',spec);await killed.ready();killed.child.kill('SIGKILL');evidence.killed={exit:await killed.closed,...killed.snapshot()};
 const cleanup=Bun.spawnSync([binary,'box','down','pd-consumer-killed']);evidence.killCleanup={code:cleanup.exitCode};
}finally{await box.stop();writeFileSync('/private/tmp/pd-1015/evidence/consumer.json',JSON.stringify(evidence,null,2));}
console.log(JSON.stringify(evidence,null,2));
