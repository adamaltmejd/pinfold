#!/usr/bin/env python3
"""Real Chromium observations and a small stdlib CDP client."""
import base64
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import socket
import struct
import subprocess
import sys
import time
import urllib.request

CASE = sys.argv[1]
WORK = Path('/workspace')
DIRECT = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def emit(kind, value):
    print(json.dumps({'kind': kind, 'value': value}), flush=True)

class CDP:
    def __init__(self, url):
        target = urllib.parse.urlsplit(url)
        self.sock = socket.create_connection((target.hostname, target.port), timeout=30)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f'GET {target.path} HTTP/1.1\r\nHost: {target.netloc}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n').encode())
        response = b''
        while not response.endswith(b'\r\n\r\n'):
            response += self.sock.recv(1)
        accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest())
        if b'101 Switching Protocols' not in response or accept not in response:
            raise RuntimeError(response.decode())
        self.ident = 0
        self.events = []

    def read(self, n):
        data = b''
        while len(data) < n:
            block = self.sock.recv(n - len(data))
            if not block:
                raise EOFError('CDP closed')
            data += block
        return data

    def recv(self):
        first, second = self.read(2)
        n = second & 127
        if n == 126:
            n = struct.unpack('!H', self.read(2))[0]
        elif n == 127:
            n = struct.unpack('!Q', self.read(8))[0]
        if second & 128:
            raise RuntimeError('unexpected masked server frame')
        data = self.read(n)
        if first & 15 != 1:
            raise RuntimeError(f'unexpected websocket opcode {first & 15}')
        return json.loads(data)

    def call(self, method, params=None):
        self.ident += 1
        data = json.dumps({'id': self.ident, 'method': method, 'params': params or {}}).encode()
        n = len(data)
        header = bytes([129, 128 | n]) if n < 126 else bytes([129, 254]) + struct.pack('!H', n)
        mask = os.urandom(4)
        self.sock.sendall(header + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))
        while True:
            message = self.recv()
            if message.get('id') == self.ident:
                if 'error' in message:
                    raise RuntimeError(message)
                return message.get('result', {})
            self.events.append(message)

    def load(self, url):
        self.events = []
        result = self.call('Page.navigate', {'url': url})
        emit('navigate', result)
        while not any(e.get('method') == 'Page.loadEventFired' for e in self.events):
            self.events.append(self.recv())
        emit('document', self.call('Runtime.evaluate', {'expression': 'document.body.innerText', 'returnByValue': True}))
        emit('network', [e for e in self.events if e.get('method') in ['Network.loadingFailed', 'Network.responseReceived']])

flags = ['--headless', '--no-sandbox', f'--user-data-dir=/home/user/{CASE}', '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=9222', f'--log-net-log=/workspace/{CASE}-netlog.json', '--net-log-capture-mode=Everything']
if CASE != 'env-only':
    flags += ['--proxy-server=http://127.0.0.1:3128']
command = ['strace', '-f', '-s', '256', '-e', 'trace=network', '-o', f'/workspace/{CASE}-strace.txt', '/usr/bin/chromium', *flags, 'about:blank']
emit('command', command)
start = time.monotonic()
proc = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
stderr = []
try:
    deadline = start + 40
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([proc.stderr], [], [], remaining)[0]:
            raise TimeoutError('no DevTools readiness')
        line = proc.stderr.readline()
        stderr.append(line)
        if not line:
            raise RuntimeError(f'Chrome exited before readiness: {proc.poll()}')
        if 'DevTools listening on ' in line:
            break
    emit('ready_seconds', round(time.monotonic() - start, 3))
    version = json.load(DIRECT.open('http://127.0.0.1:9222/json/version'))
    emit('devtools_version', version)
    page = json.load(DIRECT.open(urllib.request.Request('http://127.0.0.1:9222/json/new?about:blank', method='PUT')))
    cdp = CDP(page['webSocketDebuggerUrl'])
    cdp.call('Page.enable')
    cdp.call('Network.enable')
    processes = []
    for p in Path('/proc').iterdir():
        if p.name.isdigit():
            try:
                cmd = (p / 'cmdline').read_bytes().replace(b'\x00', b' ').decode()
                if '/usr/lib/chromium/chromium' in cmd:
                    status = [line for line in (p / 'status').read_text().splitlines() if line.split(':')[0] in ['Name', 'Uid', 'Gid', 'CapInh', 'CapPrm', 'CapEff', 'CapBnd', 'CapAmb', 'NoNewPrivs']]
                    processes.append({'pid': p.name, 'command': cmd, 'status': status})
            except (OSError, UnicodeError):
                pass
    emit('process_controls', processes)
    emit('interfaces', sorted(p.name for p in Path('/sys/class/net').iterdir()))
    emit('root_mount', [line for line in Path('/proc/mounts').read_text().splitlines() if line.split()[1] == '/'])
    emit('user_max_user_namespaces', Path('/proc/sys/user/max_user_namespaces').read_text().strip())
    try:
        Path('/probe-chrome-write').write_text('fixture')
        emit('root_write', 'unexpected success')
    except OSError as e:
        emit('root_write', str(e))
    cdp.load('https://api.github.com/')
    if CASE == 'explicit-proxy':
        cdp.load('https://example.com/')
        # Plain HTTP exposes the actual proxy 403 in the browser as well.
        cdp.load('http://example.com/')
        cdp.load('file:///workspace/fixture.html')
        emit('rendered_state', cdp.call('Runtime.evaluate', {'expression': 'document.getElementById("result").textContent', 'returnByValue': True}))
        png = cdp.call('Page.captureScreenshot', {'format': 'png', 'captureBeyondViewport': True})
        (WORK / 'rendered.png').write_bytes(base64.b64decode(png['data']))
        pdf = cdp.call('Page.printToPDF', {'printBackground': True})
        (WORK / 'rendered.pdf').write_bytes(base64.b64decode(pdf['data']))
        emit('webrtc', cdp.call('Runtime.evaluate', {'expression': '''(async () => {
          const pc = new RTCPeerConnection({iceServers:[{urls:'stun:example.com:3478'}]});
          pc.createDataChannel('fixture');
          const events=[];
          pc.onicecandidateerror=e=>events.push({code:e.errorCode,text:e.errorText,url:e.url});
          await pc.setLocalDescription(await pc.createOffer());
          await new Promise(resolve=>{pc.onicegatheringstatechange=()=>{if(pc.iceGatheringState==='complete')resolve()};setTimeout(resolve,12000)});
          const result={state:pc.iceGatheringState,events,sdp:pc.localDescription.sdp};pc.close();return result;
        })()''', 'awaitPromise': True, 'returnByValue': True}))
        # Keep ordinary background services running during a bounded observation.
        emit('startup_observation', cdp.call('Runtime.evaluate', {'expression': 'new Promise(resolve=>setTimeout(()=>resolve("60 seconds of background observation complete"),60000))', 'awaitPromise': True, 'returnByValue': True}))
    cdp.call('Browser.close')
    cdp.sock.close()
    tail = proc.communicate(timeout=20)
    stderr.extend(tail[1].splitlines(keepends=True))
    emit('chrome_exit', proc.returncode)
except Exception as error:
    emit('error', repr(error))
    os.killpg(proc.pid, signal.SIGTERM)
    try:
        stderr.append(proc.communicate(timeout=15)[1])
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        stderr.append(proc.communicate()[1])
    raise
finally:
    (WORK / f'{CASE}-stderr.txt').write_text(''.join(stderr))
    emit('elapsed_seconds', round(time.monotonic() - start, 3))
