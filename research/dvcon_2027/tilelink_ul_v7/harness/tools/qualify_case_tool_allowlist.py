"""Live, fresh-thread transport qualification; not a debugging subject."""
import argparse,hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'agents'))
from codex_app_server import CodexAppServer,cached_token_observations
p=argparse.ArgumentParser();p.add_argument('--codex-bin',required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args();out=a.out.resolve();out.mkdir(exist_ok=False);work=out/'workspace';work.mkdir();receipt={'status':'FAIL','purpose':'case-only tool exposure and transport qualification; not diagnosis/efficacy'};calls=[]
tool={'name':'read_evidence','description':'Return the sole case qualification marker.','inputSchema':{'type':'object','properties':{},'required':[],'additionalProperties':False}}
def handler(name,arguments):
 if name!='read_evidence' or arguments!={}:raise ValueError('unexpected qualification call')
 calls.append(name);return {'marker':'TLUL_CASE_BOUNDARY_OK','citation_id':'qualification_evidence_1'}
try:
 # Deliberately no workspace constructor: this engineering qualification bootstraps
 # the receipt needed for real subjects. The thread itself retains the same profile.
 with CodexAppServer(codex=a.codex_bin,stderr_path=out/'stderr.log',event_path=out/'events.jsonl') as app:
  app.initialize();receipt['preflight']=app.preflight();app.handler=handler;thread=app.start_thread(work,'gpt-6.1-sol',[tool]);receipt['fresh_thread_created']=True
  answer=app.turn(thread_id=thread,cwd=work,model='gpt-6.1-sol',effort='medium',text='This is a tool-transport qualification, not a debugging case. Call read_evidence with {} exactly once and report its marker and citation_id in a JSON object. Use only the registered tool.',timeout=180)
  receipt['surface']=app.verify_tool_surface();receipt['calls']=calls;receipt['usage']=cached_token_observations(app.events);receipt['violations']=app.violation;receipt['status']='PASS' if calls==['read_evidence'] and 'TLUL_CASE_BOUNDARY_OK' in answer['text'] and not app.violation else 'FAIL'
except Exception as e:receipt['error']=type(e).__name__+': '+str(e)
if 'app' in globals():
 receipt['usage']=cached_token_observations(app.events)
 receipt['calls']=calls
 receipt['violations']=app.violation
receipt['codex_sha256']=hashlib.sha256(Path(a.codex_bin).resolve().read_bytes()).hexdigest();(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps({k:v for k,v in receipt.items() if k!='usage'},indent=2));raise SystemExit(0 if receipt['status']=='PASS' else 1)
