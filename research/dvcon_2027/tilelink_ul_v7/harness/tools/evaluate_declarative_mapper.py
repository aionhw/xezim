#!/usr/bin/env python3
"""Execute a bounded declarative mapping proposal against independent PIN truth."""
import argparse,hashlib,json
from pathlib import Path
from check_pins import parse,evaluate
from semantic_adapter import summarize
ROOT=Path(__file__).resolve().parents[1]
FIELDS={'cycle':'cycle','a_valid':'av','a_ready':'ar','a_source':'asrc','a_opcode':'aop','a_address':'addr','a_size':'asz','a_mask':'mask','d_valid':'dv','d_ready':'dr','d_source':'dsrc','d_opcode':'dop','d_data':'ddata'}
def map_events(samples,rule):
 def accepted(s,kind):return all(s.get(FIELDS[f])==1 for f in rule['acceptance'][kind])
 pending={};ambiguous=set();pairs=[];unknown=[];ordinal=0
 for s in samples:
  aa=accepted(s,'request');dd=accepted(s,'response');ak=s[FIELDS[rule['request_key']]];dk=s[FIELDS[rule['response_key']]];old=None;was_ambiguous=dk in ambiguous
  if dd:
   old=pending.pop(dk,None)
   if dk in ambiguous:old=None;ambiguous.discard(dk)
  new=None
  if aa:ordinal+=1;new=(ordinal,s['cycle'],ak)
  if dd:
   if old is None and new and new[2]==dk and not was_ambiguous:old=new;new=None
   if old is None:unknown.append({'cycle':s['cycle'],'kind':'orphan_or_ambiguous_d'})
   else:pairs.append((s['cycle'],dk,old[1],old[0]))
  if new:
   if ak in pending or ak in ambiguous:
    pending.pop(ak,None);ambiguous.add(ak);unknown.append({'cycle':s['cycle'],'kind':'duplicate_outstanding'})
   else:pending[ak]=new
 return {'pairs':pairs,'unknown':unknown,'pending':len(pending),'ambiguous_sources':sorted(ambiguous)}
def main():
 p=argparse.ArgumentParser();p.add_argument('--receipt',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args();a.out.mkdir(exist_ok=False);r=json.loads(a.receipt.read_text())
 if not r.get('status','').startswith('COMPLETED') or r.get('preflight',{}).get('auth_type')!='chatgpt' or r.get('preflight',{}).get('model')!='gpt-6.1-sol' or r.get('preflight',{}).get('effort')!='medium' or r.get('tool_surface_audit',{}).get('allowed_tools')!=[]:raise ValueError('actual strict subscription mapper receipt required')
 rule=json.loads(r['reply']['text']);required={'request_key':'a_source','response_key':'d_source','simultaneous':'retire_prior_then_insert_or_match_new_if_empty','duplicate_outstanding':'ambiguous_unknown','orphan_response':'unknown','provenance':'unknown'}
 valid=all(rule.get(k)==v for k,v in required.items()) and all(isinstance(rule.get('acceptance',{}).get(k),list) and len(rule['acceptance'][k])==2 and set(rule['acceptance'][k])==v for k,v in {'request':{'a_valid','a_ready'},'response':{'d_valid','d_ready'}}.items())
 results={};controls={}
 if valid:
  for name in ('seed_123','seed_9876','zero_2026'):
   log=(ROOT/f'results/clean_admission/reference_{name}.log').read_text();meta=json.loads((ROOT/f'stimulus/{name}/stimulus.json').read_text());oracle=evaluate(log,meta);samples,_=parse(log);mapped=map_events(samples,rule);fixed=summarize(log,10000);truth=[(e['cycle'],e['source'],e['matched_a_cycle'],e['matched_request_ordinal']) for e in fixed['events'] if e['kind']=='accepted_d'];results[name]={'oracle':oracle['status'],'joined_responses':len(mapped['pairs']),'wrong_or_unmatched':len(mapped['unknown']),'pending':mapped['pending'],'equal_fixed_mapping':mapped['pairs']==truth,'eligible_d':oracle['d_accepted']}
  def sample(cycle,av=0,src=1,dv=0):return {'cycle':cycle,'av':av,'ar':1,'asrc':src,'dv':dv,'dr':1,'dsrc':src}
  orphan=map_events([sample(1,dv=1)],rule);duplicate=map_events([sample(1,av=1),sample(2,av=1),sample(3,dv=1)],rule);reuse=map_events([sample(1,av=1),sample(2,av=1,dv=1),sample(3,dv=1)],rule);incomplete=map_events([sample(1,av=1)],rule)
  controls={'missing_request_remains_unknown':not orphan['pairs'] and len(orphan['unknown'])==1,'duplicate_source_remains_unknown':not duplicate['pairs'] and len(duplicate['unknown'])==2,'same_cycle_reuse_matches_prior_first':reuse['pairs']==[(2,1,1,1),(3,1,2,2)],'incomplete_request_remains_pending':incomplete['pending']==1 and not incomplete['pairs']}
 good=valid and all(x['oracle']=='PASS' and x['joined_responses']==x['eligible_d'] and not x['wrong_or_unmatched'] and not x['pending'] and x['equal_fixed_mapping'] for x in results.values()) and all(controls.values())
 receipt={'status':'PASS_BOUNDED_SCHEDULES_AND_NEGATIVE_CONTROLS' if good else 'FAIL','proposal_schema_valid':valid,'proposal_sha256':hashlib.sha256(r['reply']['text'].encode()).hexdigest(),'cases':results,'controls':controls,'scope':'model-chosen declarative rules interpreted by trusted host; external event mapping, not a native plugin ABI; bounded integer-valued sample tests'}
 (a.out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2));return 0 if good else 1
if __name__=='__main__':raise SystemExit(main())
