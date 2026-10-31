#!/usr/bin/env python3
"""Host operationalization of a prose JSON proposal; no execution of model code."""
import json,hashlib
from pathlib import Path
from check_pins import parse,evaluate
from semantic_adapter import summarize
ROOT=Path(__file__).resolve().parents[1];out=ROOT/'results/mapper_evaluation_001';out.mkdir(exist_ok=False)
r=json.loads((ROOT/'results/mapper_retained_profile_fix_001/receipt.json').read_text());proposal=json.loads(r['reply']['text']);required={'request':'a_valid == 1 and a_ready == 1','response':'d_valid == 1 and d_ready == 1'}
assert all(proposal['acceptance'][k]==v for k,v in required.items());assert proposal['state']['outstanding']['key']=='a_source';assert proposal['matching']['key']=='d_source == retained_request.a_source'
results={}
for name in ('seed_123','seed_9876','zero_2026'):
 log=(ROOT/f'results/clean_admission/reference_{name}.log').read_text();meta=json.loads((ROOT/f'stimulus/{name}/stimulus.json').read_text());oracle=evaluate(log,meta);assert oracle['status']=='PASS'
 samples,_=parse(log);pending={};pairs=[];errors=[];ordinal=0
 # Host translation of the model's prior-outstanding-first lifetime semantics.
 for s in samples:
  accepted_a=s['av']==s['ar']==1;accepted_d=s['dv']==s['dr']==1;old=pending.pop(s['dsrc'],None) if accepted_d else None
  new=None
  if accepted_a:ordinal+=1;new=(ordinal,s['cycle'],s['asrc'])
  if accepted_d:
   if old is None and new and new[2]==s['dsrc']:old=new;new=None
   if old is None:errors.append({'cycle':s['cycle'],'reason':'unmatched_d'})
   else:pairs.append((s['cycle'],s['dsrc'],old[1],old[0]))
  if new:
   if new[2] in pending:errors.append({'cycle':s['cycle'],'reason':'occupied_source'})
   else:pending[new[2]]=new
 fixed=summarize(log,10000);fixed_pairs=[(e['cycle'],e['source'],e['matched_a_cycle'],e['matched_request_ordinal']) for e in fixed['events'] if e['kind']=='accepted_d']
 results[name]={'oracle':'PASS','joined_responses':len(pairs),'wrong_or_unmatched_joins':len(errors),'pending':len(pending),'fixed_mapping_equal':pairs==fixed_pairs,'eligible_d':oracle['d_accepted']}
receipt={'status':'PASS_BOUNDED_SCHEDULES' if all(x['joined_responses']==x['eligible_d'] and not x['wrong_or_unmatched_joins'] and not x['pending'] and x['fixed_mapping_equal'] for x in results.values()) else 'FAIL','scope':'host-written operationalization of model prose JSON; not a model-generated executable adapter, not native observer data','proposal_sha256':hashlib.sha256(r['reply']['text'].encode()).hexdigest(),'cases':results,'limits':'Does not qualify unknown/missing samples, ambiguous histories or every same-cycle reuse scenario.'}
(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2))
