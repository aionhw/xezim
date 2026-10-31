#!/usr/bin/env python3
"""Fault-cluster paired bootstrap for a complete, independently adjudicated cohort.

Input must contain all scheduled rows, including failed subjects scored false.
No pilot or partially adjudicated cohort is analyzed as primary evidence.
"""
import argparse,json,random,statistics
from pathlib import Path

def analyze(data):
 if data.get('cohort')!='primary' or data.get('independent_review_complete') is not True:
  raise ValueError('complete independent primary review required')
 cases=data['cases'];rows=data['rows'];faults=[c for c,v in cases.items() if v=='fault'];clean=[c for c,v in cases.items() if v=='clean']
 if len(faults)!=12 or len(clean)!=2 or len(cases)!=14:raise ValueError('expected twelve faults and two clean controls')
 expected={(c,m,r) for c in cases for m in 'ABCD' for r in (1,2,3)};index={}
 for row in rows:
  key=(row['case'],row['mode'],row['repeat'])
  if key in index:raise ValueError('duplicate scheduled subject')
  if row.get('strict_eligible') is not True:raise ValueError('unqualified subject cannot enter primary analysis')
  for k in ('stage2_runtime_verified', 'stage1_component_and_mechanism_correct' if row['case'] in faults else 'clean_assessment_correct'):
   if type(row.get(k)) is not bool:raise ValueError('explicit adjudication required; unknown is not false')
  index[key]=row
 if set(index)!=expected:raise ValueError('all 168 scheduled outcomes required')
 def mean(case,mode,key):return statistics.mean(index[case,mode,r][key] for r in (1,2,3))
 contrasts={}
 for newer,older in [('C','B'),('D','C')]:
  diffs=[mean(c,newer,'stage1_component_and_mechanism_correct')-mean(c,older,'stage1_component_and_mechanism_correct') for c in faults]
  rng=random.Random(71032026);boot=sorted(statistics.mean(rng.choices(diffs,k=12)) for _ in range(10000))
  contrasts[newer+'-'+older]={'estimate':statistics.mean(diffs),'percentile_95_interval':[boot[249],boot[9749]],'fault_clusters':12,'bootstrap_resamples':10000,'zero_width_interval':boot[249]==boot[9749],'interpretation':'A zero-width empirical interval does not demonstrate equivalence.'}
 modes={}
 for mode in 'ABCD':
  fault_rows=[index[c,mode,r] for c in faults for r in (1,2,3)];clean_rows=[index[c,mode,r] for c in clean for r in (1,2,3)]
  modes[mode]={'fault_stage1_correct':sum(r['stage1_component_and_mechanism_correct'] for r in fault_rows),'fault_denominator':36,'fault_verified_repairs':sum(r['stage2_runtime_verified'] for r in fault_rows),'clean_correct':sum(r['clean_assessment_correct'] for r in clean_rows),'clean_denominator':6}
 return {'status':'ANALYZED_PRIMARY','modes':modes,'paired_stage1_contrasts':contrasts}

def main():
 p=argparse.ArgumentParser();p.add_argument('--adjudication',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args();result=analyze(json.loads(a.adjudication.read_text()))
 with a.out.open('x') as f:json.dump(result,f,indent=2);f.write('\n')
if __name__=='__main__':main()
