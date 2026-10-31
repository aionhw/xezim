#!/usr/bin/env python3
"""Fresh, source-free subscribed model mapper proposal, engineering calibration only."""
import argparse, hashlib, json, sys, time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'agents'))
from codex_app_server import CodexAppServer, cached_token_observations
parser=argparse.ArgumentParser();parser.add_argument('--out',type=Path,required=True);parser.add_argument('--codex-bin',default=str(ROOT/json.loads((ROOT/'config/model.json').read_text())['codex_cli_path']));args=parser.parse_args()
out=args.out.resolve()
out.mkdir()
workspace=out/'workspace'; workspace.mkdir()
result={'status':'IN_PROGRESS','subject':'independent adapter generation; not a debugging trial',
 'generator_code_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
 'codex_binary_sha256':hashlib.sha256(Path(args.codex_bin).resolve().read_bytes()).hexdigest()}
try:
 with CodexAppServer(codex=args.codex_bin,stderr_path=out/'stderr.log',event_path=out/'events.jsonl',workspace=workspace) as app:
  app.initialize(); result['initialize']=app.initialize_result;result['process_id']=app.proc.pid; result['preflight']=app.preflight('gpt-6.1-sol','medium')
  thread=app.start_thread(workspace,'gpt-6.1-sol',[]); result['thread_id']=thread
  prompt=('Generate a declarative JSON mapping for accepted TileLink UL requests and responses. '
   'The only fields available at each sampled clock are cycle, a_valid, a_ready, a_source, '
   'a_opcode, a_address, a_size, a_mask, d_valid, d_ready, d_source, d_opcode, d_data. '
   'A request is accepted when a_valid and a_ready are 1. A response is accepted when '
   'd_valid and d_ready are 1. An A source is unique while outstanding, but may be reused '
   'after retirement. Do not invent values or provenance. Return JSON describing '
   'the state, matching key, insertion, removal, and unknown/missing cases. No code, '
   'no tools, no markdown. Ignore all prior cases. Use this machine-readable schema: '
   '{"acceptance":{"request":["two boolean field names"],"response":["two boolean field names"]},'
   '"request_key":"field name","response_key":"field name",'
   '"simultaneous":"choose retire_prior_then_insert_or_match_new_if_empty or insert_before_retire",'
   '"duplicate_outstanding":"choose ambiguous_unknown or overwrite",'
   '"orphan_response":"choose unknown or invent_request",'
   '"provenance":"choose unknown or inferred_writer"}. '
   'Choose the rules consistent with accepted transfers, source reuse, and unknown histories.')
  reply=app.turn(thread_id=thread,cwd=workspace,model='gpt-6.1-sol',effort='medium',text=prompt,timeout=1200)
  result['reply']=reply;result['tool_surface_audit']=app.verify_tool_surface(); result['cache_observations']=cached_token_observations(app.events);result['status']='COMPLETED_UNSCORED_PENDING_INDEPENDENT_EVALUATION'
except Exception as exc:
 result['status']='FAILED';result['error']=type(exc).__name__+': '+str(exc)
if 'app' in globals():
 result['cache_observations']=cached_token_observations(app.events)
 result['cache_telemetry_reported']=bool(result['cache_observations'])
 result['cached_tokens_if_unreported']='unknown'
(out/'receipt.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k not in ('reply','cache_observations')},indent=2))
sys.exit(0 if result['status'].startswith('COMPLETED') else 1)
