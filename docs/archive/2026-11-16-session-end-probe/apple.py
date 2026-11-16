import sys,subprocess,os
from pathlib import Path
def measure_store():
 import json, pathlib, subprocess, collections
 for kind,cmd in {'images':['image','list'],'containers':['list','--all'],'volumes':['volume','list'],'df':['system','df']}.items():
  pathlib.Path('/tmp/pfp-store-'+kind+'-now.json').write_bytes(subprocess.check_output(['container']+cmd+['--format','json']))
 base=pathlib.Path.home()/'Library/Application Support/com.apple.container'
 images=json.load(open('/tmp/pfp-store-images-now.json')); boxes=json.load(open('/tmp/pfp-store-containers-now.json'))
 groups=collections.defaultdict(list)
 for im in images: groups[im['id']].append(im)
 refs={b['configuration']['image']['descriptor']['digest'].split(':')[-1]:b['id']+':'+b['status']['state'] for b in boxes}
 def closure(digest,seen):
  if digest in seen:return
  seen.add(digest); p=base/'content/blobs/sha256'/digest
  try: obj=json.loads(p.read_bytes())
  except (OSError,ValueError):return
  for d in obj.get('manifests',[])+obj.get('layers',[])+([obj['config']] if 'config' in obj else []):closure(d['digest'].split(':')[-1],seen) if 'digest' in d else None
 def blocks(paths):return sum(p.stat().st_blocks*512 for p in paths if p.exists())
 lines=['Store snapshot: read-only container inventories; sizes bytes unless KiB named.','Command: container image list --format json; container volume list --format json; container list --all --format json; container system df --format json.','Measurement: stat.st_blocks*512 for each image OCI content closure and its unpacked snapshot(s); aliases grouped. Content and APFS clones share storage; rows are attributable allocations, not exclusive/reclaimable totals.']
 for ident,ims in groups.items():
  im=ims[0]; seen=set();closure(ident,seen)
  content=blocks([base/'content/blobs/sha256'/d for d in seen]); snaps=blocks([base/'snapshots'/v['digest'].split(':')[-1]/'snapshot' for v in im['variants']]); names=','.join(i['configuration']['name'] for i in ims)
  lines.append(f"{names} | OCI={sum(v['size'] for v in im['variants'])} content_alloc={content} snapshot_alloc={snaps} reference={refs.get(ident,'none')}")
 for v in json.load(open('/tmp/pfp-store-volumes-now.json')):
  p=pathlib.Path(v['configuration']['source']); lines.append(f"Volume {v['id']} logical={p.stat().st_size} allocated={p.stat().st_blocks*512}.")
 for sub in ['builder','containers/buildkit','content','snapshots']:
  n=subprocess.check_output(['du','-sk',str(base/sub)],text=True).split()[0];lines.append(f"du -sk /Users/user/Library/Application Support/com.apple.container/{sub}: {n} KiB")
 lines.append('container system df current: '+json.dumps(json.load(open('/tmp/pfp-store-df-now.json')),separators=(',',':')))
 lines.append('Unused means no active/stopped container image-digest reference in this snapshot; pinfold project/profile configuration and aliases may retain them. No deletion assessment or removal was performed.')
 lines.append('Builder exports directory is not the whole BuildKit cache: BuildKit data also lives in its rootfs.ext4. Whole buildkit container allocation is reported separately.')
 lines.append('Guest builder cache (du -sk): '+subprocess.check_output(['container','exec','buildkit','du','-sk','/var/lib/buildkit','/var/lib/container-builder-shim'],text=True).strip().replace('\n','; '))
 lines.append(subprocess.check_output(['df','-g','/System/Volumes/Data'],text=True).strip())
 pathlib.Path('/tmp/pfp-store-findings.txt').write_text('\n'.join(lines)+'\n')
 print('\n'.join(lines))
if "--store" in sys.argv:
 measure_store();sys.exit(0)
context=Path("/tmp/pfp-initimage");context.mkdir(exist_ok=True)
os.environ.update(ZIG_GLOBAL_CACHE_DIR=str(context/"zig-global"),ZIG_LOCAL_CACHE_DIR=str(context/"zig-local"))
subprocess.run(["zig","cc","-target","aarch64-linux-musl","-static","-Os",str(Path(__file__).with_name("vmexec.c")),"-o",str(context/"pfp-vmexec-wrapper")],check=True)
subprocess.run(["container","image","save","--output","/tmp/pfp-init-base.tar","ghcr.io/apple/containerization/vminit:0.53.0"],check=True)
import gzip,hashlib,io,json,pathlib,tarfile
base=tarfile.open('/tmp/pfp-init-base.tar'); blobs={}
def read(d):return base.extractfile('blobs/sha256/'+d['digest'].split(':')[1]).read()
def obj(d):return json.loads(read(d))
def desc(data,kind):
 h=hashlib.sha256(data).hexdigest();blobs[h]=data
 return {'digest':'sha256:'+h,'mediaType':'application/vnd.oci.image.'+kind,'size':len(data)}
index=json.load(base.extractfile('index.json')); item=index['manifests'][0]
while item['mediaType'].endswith('index.v1+json'):
 choices=obj(item)['manifests'];item=next(x for x in choices if x.get('platform',{}).get('architecture')=='arm64')
manifest=obj(item);config=obj(manifest['config']);diffs=[];original=None
for layer in manifest['layers']:
 data=read(layer);blobs[layer['digest'].split(':')[1]]=data
 raw=gzip.decompress(data) if layer['mediaType'].endswith('+gzip') else data
 diffs.append('sha256:'+hashlib.sha256(raw).hexdigest())
 with tarfile.open(fileobj=io.BytesIO(raw)) as fs:
  for member in fs.getmembers():
   if member.name.removeprefix('./')=='sbin/vmexec':original=fs.extractfile(member).read()
assert original is not None,'base layer lacks /sbin/vmexec'
layerbuf=io.BytesIO()
with tarfile.open(fileobj=layerbuf,mode='w',format=tarfile.USTAR_FORMAT) as fs:
 for name,data in [('sbin/vmexec.real',original),('sbin/vmexec',pathlib.Path('/tmp/pfp-initimage/pfp-vmexec-wrapper').read_bytes())]:
  member=tarfile.TarInfo(name);member.size=len(data);member.mode=0o755;fs.addfile(member,io.BytesIO(data))
newlayer=desc(layerbuf.getvalue(),'layer.v1.tar');diffs.append(newlayer['digest']);manifest['layers'].append(newlayer)
config['rootfs']['diff_ids']=diffs
manifest['config']=desc(json.dumps(config,separators=(',',':')).encode(),'config.v1+json')
item=desc(json.dumps(manifest,separators=(',',':')).encode(),'manifest.v1+json');item['platform']={'architecture':'arm64','os':'linux','variant':'v8'}
item['annotations']={k:'localhost/probe-s322-vminit:latest' for k in ['org.opencontainers.image.ref.name','io.containerd.image.name','com.apple.containerization.image.name']}
index={'schemaVersion':2,'mediaType':'application/vnd.oci.image.index.v1+json','manifests':[item]}
with tarfile.open('/tmp/pfp-init-probe.tar','w') as out:
 for name,data in [('oci-layout',b'{"imageLayoutVersion":"1.0.0"}'),('index.json',json.dumps(index).encode())]+[('blobs/sha256/'+h,d) for h,d in blobs.items()]:
  member=tarfile.TarInfo(name);member.size=len(data);member.mode=0o644;out.addfile(member,io.BytesIO(data))
print('base /sbin/vmexec found:',len(original),'bytes; fixed diff_ids:',diffs,'manifest:',item['digest'])
print('wrote /tmp/pfp-init-probe.tar for localhost/probe-s322-vminit:latest')

subprocess.run(["container","image","load","--input","/tmp/pfp-init-probe.tar"],check=True)
