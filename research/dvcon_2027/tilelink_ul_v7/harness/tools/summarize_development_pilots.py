"""Summarize immutable pre-allowlist pilot receipts; never confer scoring eligibility."""
import json,statistics
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def main():
 out=ROOT/'paper/receipts/development_pilots_v1.json'
 rows=[]
 for case in range(1,6):
  for mode in 'ABCD':
   tag=f'pilot_case01_{mode}_001' if case==1 else f'pilot_case_{case:02d}_{mode}_001'
   p=ROOT/'results'/tag;d=json.loads((p/'trial.json').read_text());gpath=ROOT/'results'/(tag+'_grade')/'grade.json';g=json.loads(gpath.read_text()) if gpath.exists() else {}
   usage=d.get('cache_observations',[]);total=usage[-1].get('tokenUsage',{}).get('total',{}) if usage else {}
   sim=[json.loads(q.read_text()) for q in p.glob('experiment_*/*/receipt.json')]
   rows.append({'incremental_simulator_runs':len(sim),'incremental_simulator_wall_seconds':sum(q.get('wall_seconds',0) for q in sim),'case':f'pilot_{case:02d}','mode':mode,'status':d['status'],'grade':g.get('status','NOT_RUN'),'stage1_sha256':d.get('stages',{}).get('I',{}).get('sha256'),'elapsed_seconds':d.get('elapsed_seconds'),'actions':len(d.get('actions',[])),'returned_bytes':sum(x.get('returned_bytes',0) for x in d.get('actions',[])),'input_tokens':total.get('inputTokens'),'output_tokens':total.get('outputTokens'),'cached_input_tokens':total.get('cachedInputTokens'),'eligibility':'DEVELOPMENT_ONLY_TOOL_SURFACE_UNQUALIFIED'})
 modes={}
 for mode in 'ABCD':
  r=[x for x in rows if x['mode']==mode]
  modes[mode]={'subjects':len(r),'completed':sum(x['status']=='COMPLETED' for x in r),'runtime_verified_repairs':sum(x['grade']=='VERIFIED_REPAIR' for x in r),'median_wall_seconds':statistics.median(x['elapsed_seconds'] for x in r),'total_input_tokens':sum(x['input_tokens'] for x in r if x['input_tokens'] is not None),'total_output_tokens':sum(x['output_tokens'] for x in r if x['output_tokens'] is not None),'total_cached_input_tokens':sum(x['cached_input_tokens'] for x in r if x['cached_input_tokens'] is not None),'usage_unknown_subjects':sum(x['input_tokens'] is None for x in r),'total_tool_evidence_bytes':sum(x['returned_bytes'] for x in r),'incremental_simulator_runs':sum(x['incremental_simulator_runs'] for x in r),'incremental_simulator_wall_seconds':sum(x['incremental_simulator_wall_seconds'] for x in r)}
 result={'status':'DEVELOPMENT_ONLY','exclusion':'Stock runner advertises non-case built-in utility tools. No forbidden call observed; strict requested exposure boundary was not established. All twenty subjects excluded uniformly. Cached tokens are telemetry and not an exclusion reason.','model':'gpt-6.1-sol','effort':'medium','modes':modes,'subjects':rows,'primary_sessions':0,'primary_stage1_expert_scores':'NOT_RUN'}
 out.parent.mkdir(exist_ok=True,parents=True);out.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(modes,indent=2))
if __name__=='__main__':main()
