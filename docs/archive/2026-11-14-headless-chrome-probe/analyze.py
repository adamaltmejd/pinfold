#!/usr/bin/env python3
"""Summarize retained logs without inferring traffic from URL strings alone."""
from collections import Counter
import json
import gzip
from pathlib import Path
import re
from urllib.parse import urlsplit

OUT = Path(__file__).resolve().parent
net = json.loads((OUT/'work/explicit-proxy-netlog.json').read_text())
types = {v:k for k,v in net['constants']['logEventTypes'].items()}
summary = {'egress': {}, 'url_requests': [], 'network_events': {}, 'external_syscalls': []}
for name in ['egress.jsonl', 'env-only-egress.jsonl', 'minimal-egress.jsonl']:
    counter = Counter((d['host'], d['decision'], d['reason']) for d in map(json.loads,(OUT/name).read_text().splitlines()))
    summary['egress'][name] = [{'host':h,'decision':d,'reason':r,'count':n} for (h,d,r),n in sorted(counter.items())]
counts = Counter(types[e['type']] for e in net['events'])
summary['network_events'] = {k:v for k,v in sorted(counts.items()) if any(x in k for x in ['DNS','RESOLVER','QUIC','UDP','PROXY','WEBRTC'])}
for event in net['events']:
    kind = types[event['type']]
    params = event.get('params', {})
    if kind == 'URL_REQUEST_START_JOB':
        url = params.get('url','')
        if url.startswith(('http:', 'https:')):
            summary['url_requests'].append({'source':event['source']['id'], 'host':urlsplit(url).hostname, 'url':url, 'method':params.get('method')})
    if kind in ['DNS_CONFIG_CHANGED','HOST_RESOLVER_MANAGER_DNS_TASK','DNS_TRANSACTION','UDP_CONNECT','QUIC_SESSION']:
        summary.setdefault('selected_events', []).append({'type':kind, 'params':params, 'phase':event['phase']})
trace_file = OUT/'work/explicit-proxy-strace.txt'
trace = (trace_file.read_text() if trace_file.exists() else gzip.decompress(trace_file.with_suffix('.txt.gz').read_bytes()).decode()).splitlines()
for line in trace:
    if any(call in line for call in ['connect(', 'sendto(', 'sendmsg(']) and 'AF_INET' in line and not re.search(r'(inet_addr\("127\.|inet_pton\(AF_INET6, "::1")', line):
        summary['external_syscalls'].append(line)
summary['loopback_proxy_connects'] = sum('connect(' in line and 'htons(3128)' in line for line in trace)
summary['internet_socket_calls'] = [line for line in trace if 'socket(AF_INET' in line]
(OUT/'analysis.json').write_text(json.dumps(summary, indent=2)+'\n')
print(json.dumps(summary, indent=2))
