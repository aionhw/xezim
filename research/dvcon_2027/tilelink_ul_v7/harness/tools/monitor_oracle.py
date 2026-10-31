"""Independent validation of public monitor claims against accepted PIN truth."""
import json
from check_pins import evaluate as evaluate_pins,parse

def evaluate(log,manifest):
 r=evaluate_pins(log,manifest)
 if not manifest.get('check_monitor_claims'):return r
 samples,_=parse(log);cycles={s['cycle'] for s in samples};claims=[];errors=[]
 for line in log.splitlines():
  if not line.startswith('MON|'):continue
  try:obj={k:int(v) for k,v in (x.split('=') for x in line.split('|')[1:])}
  except ValueError:errors.append({'kind':'invalid_monitor_claim'});continue
  claims.append(obj)
 if {c.get('cycle') for c in claims}!=cycles or len(claims)!=len(samples):errors.append({'kind':'missing_or_duplicate_monitor_samples'})
 if r['status']=='PASS':
  for c in claims:
   for field in ('source_alarm','data_alarm'):
    if c.get(field)!=0:errors.append({'kind':'monitor_false_alarm','cycle':c.get('cycle'),'component':field,'observed_alarm':c.get(field)})
 r['monitor_claim_count']=len(claims);r['monitor_alarm_count']=sum(c.get('source_alarm',0)!=0 or c.get('data_alarm',0)!=0 for c in claims)
 r['issues']=(r['issues']+errors)[:150];r['total_issue_count']+=len(errors)
 if errors:r['status']='FAIL'
 return r
