"""Run authorized live graph workers with GPT-6 Luna/low in a disposable fixture.

Usage: python scripts/eval-live-graph.py <davinci-executable> <new-artifact-dir>
Uses existing OpenAI Codex credentials; only their temporary copy is removed.
Retains fixture journals for verification, never credentials, in the artifact dir.
"""
import live_auth
import collections
import json, os, subprocess, sys, tempfile, time
from pathlib import Path

EXE = Path(sys.argv[1]).resolve()
OUT = Path(sys.argv[2]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
WORK = OUT / 'live-graph-work'
WORK.mkdir(exist_ok=True)
(WORK / '.davinci').mkdir(exist_ok=True)
(WORK / 'sum.js').write_text('exports.sum = (a, b) => a - b;\n')
(WORK / 'test.js').write_text("const assert=require('node:assert/strict'); const {sum}=require('./sum');\nassert.equal(sum(2,3),5); assert.equal(sum(-2,3),1); assert.equal(sum(0,0),0); console.log('SUM_OK');\n")
original_test = (WORK / 'test.js').read_bytes()
assert subprocess.run(['node','test.js'],cwd=WORK,capture_output=True).returncode != 0
(WORK / 'package.json').write_text(json.dumps({'private':True, 'scripts':{'test':'node test.js'}}))
(WORK / '.davinci/graph.json').write_text(json.dumps({
    'models':{r:'openai-codex/gpt-6-luna' for r in ['classifier','researcher','test-analyzer','historian','planner','writer','reviewer']},
    'budgets':{'maxResearchers':2,'maxParallelWorkers':2,'maxWorkers':16,'maxRevisionCycles':2,'maxReplans':1},
    'verifyCommands':[{'name':'sum-tests','command':'node test.js'}]}))
with tempfile.TemporaryDirectory(prefix='davinci-live-graph-auth-') as temporary:
    config = Path(temporary)
    auth = config / 'auth.json'
    lent = live_auth.lend(auth)
    (config / 'settings.json').write_text(json.dumps({'maxModelTurns':12,'defaultThinkingLevel':'low'}))
    (config / 'vector-memory.json').write_text(json.dumps({'enabled':False}))
    env = {k:v for k,v in os.environ.items() if not k.startswith(('PI_', 'DAVINCI_'))}
    env.update(PI_CODING_AGENT_DIR=str(config), DAVINCI_CODING_AGENT_DIR=str(config),
               PI_LEARNING_DISABLE_BACKGROUND='1', PI_MEMORY_ENABLED='0', PI_GRAPH_ECONOMY_MODEL='off')
    cmd = [str(EXE), '--no-session','--no-extensions','--no-mcp',
           '--no-skills','--no-context-files','--no-prompt-templates','--provider','openai-codex',
           '--model','gpt-6-luna','--thinking','low','--permission-mode','always-approve',
           '--mode','json','-p','-a',
           'This is an authorized test of the graph engine in a disposable fixture. '
           'Use tool_search to discover graph_run, then invoke graph_run exactly once with mode standard, '
           'dryRun false, and goal: Fix sum.js so sum adds its two arguments; preserve all assertions in '
           'test.js and verify with node test.js. Use the graph workers for this test; do not edit files '
           'yourself or use any other delegation tool. After graph_run returns summarize its actual outcome.']
    start = time.monotonic()
    try:
        with (OUT/'live-graph.jsonl').open('w',encoding='utf-8') as stdout, (OUT/'live-graph.stderr').open('w',encoding='utf-8') as stderr:
            rc = subprocess.run(cmd,cwd=WORK,env=env,stdout=stdout,stderr=stderr,timeout=1800).returncode
        test = subprocess.run(['node','test.js'],cwd=WORK,capture_output=True,text=True,encoding='utf-8')
        states = [json.loads(p.read_text(encoding='utf-8')) for p in (WORK/'.davinci/graph/runs').glob('*/state.json')]
        summary = {'exit':rc,'model':'gpt-6-luna','effort':'low','seconds':round(time.monotonic()-start,2),
                   'fixture_test_exit':test.returncode,'fixture_test_output':test.stdout,'runs':states}
        (OUT/'live-graph-result.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
        print(json.dumps({k:v for k,v in summary.items() if k!='runs'}),flush=True)
        print(json.dumps([{'runId':r.get('runId'),'phase':r.get('phase')} for r in states]),flush=True)
        assert rc == test.returncode == 0, summary
        assert (WORK / 'test.js').read_bytes() == original_test, 'graph changed its acceptance test'
        assert len(states) == 1 and states[0]['phase'] == 'done', states
        assert all(t['status'] == 'succeeded' for t in states[0]['tasks']), states
        efforts, models, counts = collections.Counter(), collections.Counter(), collections.Counter()
        for path in WORK.rglob('*.jsonl'):
            for line in path.read_text(encoding='utf-8').splitlines():
                if not line.strip():
                    continue
                event = json.loads(line)  # Includes the concurrent tool-event ledgers.
                counts['event_records' if path.name.endswith('.events.jsonl') else 'journal_records'] += 1
                if event.get('customType') == 'openai_responses_native_turn_v1':
                    response = event['data']['turn']['output']['finalResponse']
                    efforts[response.get('reasoning',{}).get('effort')] += 1
                    models[response.get('model')] += 1
        assert set(efforts) == {'low'} and set(models) == {'gpt-6-luna'}, (efforts, models)
        assert counts['event_records'] > 0, counts
        verified = dict(phase=states[0]['phase'], seconds=summary['seconds'],
            models=dict(models), efforts=dict(efforts), records=dict(counts),
            roles=[t['role'] for t in states[0]['tasks']])
        (OUT/'verified-summary.json').write_text(json.dumps(verified,indent=2),encoding='utf-8')
        print(json.dumps(verified),flush=True)
    finally:
        live_auth.give_back(auth, lent)
        auth.unlink(missing_ok=True)
