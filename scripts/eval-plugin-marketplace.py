"""Exercise public marketplace install/update/uninstall using isolated local state.

Usage: python scripts/eval-plugin-marketplace.py <davinci-executable> <new-artifact-dir>
Reads the official public catalog. Does not execute plugin commands or approve hooks.
The artifact dir retains the isolated configuration and public checkout evidence.
"""
import json, os, subprocess, sys, time
from pathlib import Path

EXE = Path(sys.argv[1]).resolve()
OUT = Path(sys.argv[2]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
WORK = OUT / 'marketplace-work'
WORK.mkdir(exist_ok=False)
CONFIG = WORK/'config'
CONFIG.mkdir()
env = {k:v for k,v in os.environ.items() if not k.startswith(('PI_','DAVINCI_'))}
env.update(PI_CODING_AGENT_DIR=str(CONFIG),DAVINCI_CODING_AGENT_DIR=str(CONFIG),
    HOME=str(WORK/'home'),USERPROFILE=str(WORK/'home'),CODEX_HOME=str(WORK/'home/.codex'),
    CLAUDE_CONFIG_DIR=str(WORK/'home/.claude'),GIT_TERMINAL_PROMPT='0')
rows=[]
def run(args, expect):
    start=time.monotonic()
    completed=subprocess.run([str(EXE),'plugin',*args],
        cwd=WORK,env=env,capture_output=True,text=True,encoding='utf-8',timeout=180)
    row=dict(command=args,exit=completed.returncode,seconds=round(time.monotonic()-start,2),
        stdout=completed.stdout,stderr=completed.stderr)
    rows.append(row)
    (OUT/'marketplace-result.json').write_text(json.dumps(rows,indent=2))
    assert completed.returncode==0,row
    assert expect in completed.stdout,row
    print(json.dumps(dict(command=args,exit=completed.returncode)),flush=True)

run(['marketplace','add','anthropics/claude-plugins-official'],'Added marketplace claude-plugins-official')
run(['browse','code-review'],'code-review@claude-plugins-official')
run(['install','code-review@claude-plugins-official'],'Installed code-review@claude-plugins-official')
installed=json.loads((CONFIG/'plugins/installed.json').read_text())
record=installed['plugins']['code-review@claude-plugins-official']
path=Path(record['installPath'])
assert path.resolve().is_relative_to(CONFIG.resolve()) and path.is_dir()
assert record['enabled']
run(['info','code-review'],'code-review')
run(['disable','code-review'],'Disabled')
run(['list'],'disabled')
run(['enable','code-review'],'Enabled')
run(['update','code-review'],'Installed code-review@claude-plugins-official')
run(['uninstall','code-review'],'Uninstalled code-review@claude-plugins-official')
run(['list'],'No plugins installed')
run(['marketplace','remove','claude-plugins-official'],'Removed marketplace')
assert not json.loads((CONFIG/'plugins/installed.json').read_text())['plugins']
print('MARKETPLACE_OK',flush=True)
