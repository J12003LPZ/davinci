"""Capture actual Claude Code CLI terminal cells, never candidate-generated references.

Uses an isolated HOME, a disposable repository, and an HTTP fixture on loopback.
No user account, credential, paid inference, repository secret or personal config
is used. These Linux PTY captures do not stand in for native Windows verification.
"""
import codecs
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import threading
import time

import pexpect
import pyte

VERSION = '2.1.278'
OUT = Path(os.environ['RUNNER_TEMP']) / 'reference-captures'
OUT.mkdir(parents=True, exist_ok=True)
EXE = os.environ['REFERENCE_CLI']
version = subprocess.check_output([EXE, '--version'], text=True, timeout=20).strip()
if not version.startswith(VERSION):
    raise RuntimeError(f'Wrong reference version: {version}')

class Fixture(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_GET(self):
        self.reply({'data': [{'id': 'claude-sonnet-4-6', 'type': 'model', 'display_name': 'Claude Sonnet 4.6'}], 'has_more': False})
    def do_POST(self):
        data = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        with (OUT / 'fixture-requests.jsonl').open('a') as log:
            log.write(json.dumps({'path': self.path, 'bytes': len(data)})+'\n')
        if 'count_tokens' in self.path:
            self.reply({'input_tokens': 10})
        else:
            self.reply({'id': 'msg_ui_fixture', 'type': 'message', 'role': 'assistant',
                'model': 'claude-sonnet-4-6', 'content': [{'type':'text','text':'Local UI fixture. No model inference was performed.'}],
                'stop_reason': 'end_turn', 'stop_sequence': None, 'usage': {'input_tokens':10,'output_tokens':10}})
    def reply(self, value):
        body=json.dumps(value).encode()
        self.send_response(200); self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body)

server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Fixture)
threading.Thread(target=server.serve_forever,daemon=True).start()

class Screen(pyte.Screen):
    """Respond to cursor/device queries so the real renderer can initialize."""
    child = None
    def write_process_input(self, data):
        if self.child is not None: self.child.send(data.encode())

cases = []
for theme in ['dark', 'light']:
    profile=OUT.parent / f'reference-profile-{theme}'
    home=profile/'home'; cwd=profile/'fixture-project'
    home.mkdir(parents=True,exist_ok=True); cwd.mkdir(parents=True,exist_ok=True)
    subprocess.run(['git','init','-q',str(cwd)],check=True)
    (cwd/'README.md').write_text('# UI fixture\nNo production files or credentials.\n')
    config={'hasCompletedOnboarding':True, 'lastOnboardingVersion':VERSION,
        'theme':theme, 'autoUpdates':False, 'installMethod':'native',
        'projects':{str(cwd):{'hasTrustDialogAccepted':True,'projectOnboardingSeenCount':1}}}
    (home/'.claude.json').write_text(json.dumps(config))
    configdir=home/'.claude'; configdir.mkdir(exist_ok=True)
    (configdir/'settings.json').write_text(json.dumps({'env':{'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1'}}))
    env=dict(os.environ)
    for key in list(env):
        if key.startswith(('ANTHROPIC_', 'CLAUDE_', 'AWS_', 'GOOGLE_')): env.pop(key)
    env.update({'HOME':str(home),'CLAUDE_CONFIG_DIR':str(configdir),
        'TERM':'xterm-256color','COLORTERM':'truecolor','FORCE_COLOR':'3',
        'ANTHROPIC_API_KEY':'fixture-key-not-a-credential',
        'ANTHROPIC_BASE_URL':f'http://127.0.0.1:{server.server_port}',
        'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1', 'DISABLE_AUTOUPDATER':'1',
        'DISABLE_ERROR_REPORTING':'1', 'DISABLE_TELEMETRY':'1'})
    env.pop('CI',None)
    child=pexpect.spawn(EXE,[],cwd=str(cwd),env=env,dimensions=(40,120),timeout=20,encoding=None)
    screen=Screen(120,40); screen.child=child; stream=pyte.Stream(screen)
    decoder=codecs.getincrementaldecoder('utf-8')('replace')
    raw=bytearray(); trace=[]
    def read_for(seconds):
        deadline=time.monotonic()+seconds
        while time.monotonic()<deadline and child.isalive():
            try:
                chunk=child.read_nonblocking(65536,timeout=.1)
            except pexpect.TIMEOUT: continue
            except pexpect.EOF: break
            raw.extend(chunk); stream.feed(decoder.decode(chunk))
            if b'\x1b]10;?' in chunk: child.send(b'\x1b]10;rgb:e6e6/e6e6/e6e6\x1b\\')
            if b'\x1b]11;?' in chunk: child.send(b'\x1b]11;rgb:1f1f/1f1f/1f1f\x1b\\')
    def send(data,label):
        trace.append({'input':label, 'hex':data.hex()}); child.send(data); read_for(1)
    def capture(name):
        ident=f'{theme}-{name}'
        lines=screen.display
        (OUT/(ident+'.txt')).write_text('\n'.join(lines))
        grid=[]
        for y in range(screen.lines):
            row=[]
            for x in range(screen.columns):
                cell=screen.buffer[y][x]
                row.append({'text':cell.data,'fg':cell.fg,'bg':cell.bg,'bold':cell.bold,
                    'italics':cell.italics,'underline':cell.underscore,'reverse':cell.reverse})
            grid.append(row)
        frame={'version':version,'theme':theme,'width':120,'height':40,
            'cursor':{'x':screen.cursor.x,'y':screen.cursor.y},'cells':grid,'trace':trace.copy()}
        payload=json.dumps(frame,ensure_ascii=False,indent=2).encode()
        (OUT/(ident+'.json')).write_bytes(payload)
        cases.append({'id':ident,'sha256':hashlib.sha256(payload).hexdigest(),'version':version})
        print(f'Captured {ident}: '+next((line.strip() for line in lines if line.strip()),'<empty>'))
    try:
        read_for(7); capture('initial')
        # Only advance explicit startup confirmations; no model request is sent.
        for n in range(4):
            text='\n'.join(screen.display).lower()
            if any(term in text for term in ['trust this folder','use this api key','trust the files','security notes','enter to confirm']):
                send(b'\r','confirm fixture startup'); capture(f'startup-{n}')
            else: break
        capture('welcome')
        for command in ['config','model','permissions','help','status','mcp','agents','tasks','resume']:
            send(b'\x1b','dismiss prior panel')
            send(b'\x15','clear prompt')
            send(('/'+command).encode(),'type /'+command)
            send(b'\r','submit /'+command)
            read_for(2); capture(command)
        (OUT/(theme+'-session.ansi')).write_bytes(raw)
    finally:
        child.terminate(force=True)
server.shutdown()
(OUT/'manifest.json').write_text(json.dumps({'schema_version':1,'reference':version,
    'os':platform.platform(),'terminal':'Linux PTY / pyte 0.8.2','dimensions':[120,40],
    'fixture_api':True,'account':'none','renderer':'CLI default; native Windows not captured',
    'cases':cases},indent=2))
