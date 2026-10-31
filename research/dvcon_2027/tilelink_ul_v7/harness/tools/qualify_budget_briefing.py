"""Fresh subscription helper: verify production API briefing, not diagnosis or repair."""
import argparse,hashlib,json,sys
from pathlib import Path
from types import SimpleNamespace
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'agents'))
from run_trial import Host,stage1_instructions,parse_json_answer
from codex_app_server import CodexAppServer,cached_token_observations
p=argparse.ArgumentParser();p.add_argument('--out',type=Path,required=True);a=p.parse_args();out=a.out.resolve();out.mkdir(exist_ok=False);work=out/'workspace';work.mkdir();conf=json.loads((ROOT/'config/model.json').read_text());binary=ROOT/conf['codex_cli_path'];args=SimpleNamespace(mode='A',case_manifest=None,timeout=1200,baseline=ROOT/'results/clean_admission/xezim_seed_123',fault_source=ROOT/'rtl/tlul_mem_endpoint.sv');host=Host(args,out);result={'status':'FAIL','purpose':'Post-calibration briefing and compact transport qualification; not a debugging trial or outcome retry'}
expected={'max_tool_calls':12,'max_evidence_bytes':131072,'max_trial_seconds':1200,'read_count_min':1,'read_count_max':40,'read_start_min':0,'control_at_min':4,'control_at_max':2000,'control_duration_min':1,'control_duration_max':32}
try:
 with CodexAppServer(codex=str(binary),workspace=work,stderr_path=out/'stderr.log',event_path=out/'events.jsonl') as app:
  app.initialize();result['preflight']=app.preflight('gpt-6.1-sol','medium');app.handler=host.execute;thread=app.start_thread(work,'gpt-6.1-sol',host.schemas())
  prompt=stage1_instructions(args,{'status':'PASS'})+'\n\nThis is a briefing-transport qualification, not a debugging subject. Do not diagnose or patch. Call read_evidence exactly once with kind=pin,start=0,count=1. Then return a JSON object containing the numerical limits from the briefing under these keys: '+','.join(expected)+', plus citation_id from the actual result. No implementation source will be provided.'
  reply=app.turn(thread_id=thread,cwd=work,model='gpt-6.1-sol',effort='medium',text=prompt,timeout=180);answer=parse_json_answer(reply['text']);result['surface']=app.verify_tool_surface();result['limits_recalled']={k:answer.get(k) for k in expected};result['actual_case_query_count']=len(host.actions);result['valid_issued_citation']=answer.get('citation_id') in host.issued_evidence;result['status']='PASS' if all(answer.get(k)==v for k,v in expected.items()) and len(host.actions)==1 and host.actions[0]['success'] and result['valid_issued_citation'] else 'FAIL'
except Exception as e:result['error']=type(e).__name__+': '+str(e)
if 'app' in globals():result['usage']=cached_token_observations(app.events)
result['codex_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest();result['briefing_code_sha256']=hashlib.sha256((ROOT/'agents/run_trial.py').read_bytes()).hexdigest();(out/'receipt.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k!='usage'},indent=2));raise SystemExit(0 if result['status']=='PASS' else 1)
