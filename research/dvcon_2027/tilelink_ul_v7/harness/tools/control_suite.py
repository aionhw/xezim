#!/usr/bin/env python3
"""Physically run a subject-selected control and two independent negative controls.
The second control is a no-op request_throttle=0; it is not a same-target
control when the active selected target is d_ready.

Failure is a real failure; no proxy/surrogate simulator responses are permitted.
"""
import argparse, json, sys, time
from pathlib import Path
from run_sim import run
from check_pins import parse

# The forced target is only the input under test; all independent pins remain observed.
TARGETS={'request_throttle':1,'response_backpressure':4}

def first_diff(baseline, variant, exclude=()):
    a,_=parse(Path(baseline).read_text())
    b,_=parse(Path(variant).read_text())
    bycycle={x.get('cycle'):x for x in b}
    keys=['av','ar','aop','asz','asrc','addr','mask','adata','dv','dr','dop','dsz','dsrc','ddata','derr']
    keys=[k for k in keys if k not in exclude]
    for sa in a:
        sb=bycycle.get(sa['cycle'])
        if sb is None: return {'cycle':sa['cycle'],'reason':'missing_comparable_sample'}
        ch={k:[sa[k],sb[k]] for k in keys if sa.get(k)!=sb.get(k)}
        if ch: return {'cycle':sa['cycle'],'changed_pins':ch}
    if len(a)!=len(b):return {'reason':'different_number_of_samples','baseline':len(a),'experiment':len(b)}
    ma=[line for line in Path(baseline).read_text().splitlines() if line.startswith('MON|')]
    mb=[line for line in Path(variant).read_text().splitlines() if line.startswith('MON|')]
    if ma!=mb:
        for left,right in zip(ma,mb):
            if left!=right:return {'reason':'monitor_claim_difference','baseline':left,'experiment':right}
        return {'reason':'different_number_of_monitor_samples','baseline':len(ma),'experiment':len(mb)}
    return None

def execute(xezim,source,stimuli,out,target,at,length,prediction,deadline=None,case_config=None):
    if target not in TARGETS:raise ValueError('target unavailable')
    if type(at) is not int or type(length) is not int or not 4<=at<=2000 or not 1<=length<=32:
        raise ValueError('control time/duration outside integer safe envelope')
    if not isinstance(prediction,str) or len(prediction.strip())<12:
        raise ValueError('agent must seal a nontrivial prediction before rerun')
    out=Path(out).resolve()
    if out.exists():raise FileExistsError('create-once control suite required')
    out.mkdir(parents=True)
    (out/'prediction.json').write_text(json.dumps({'target':target,'at_cycle':at,
        'duration_cycles':length,'sealed_prediction':prediction},indent=2)+'\n')
    receipts={}
    for name,op in [('baseline',0),('experiment',TARGETS[target]),('unrelated',2),('idempotent',3)]:
        remaining=240 if deadline is None else min(240,deadline-time.monotonic())
        if remaining<=0:raise TimeoutError('trial budget exhausted before control run')
        receipts[name]=run(xezim,source,stimuli,out/name,'none',op,at,length,timeout=remaining,case_config=case_config)
    base=out/'baseline'/'stdout.log'
    # d_ready itself is controlled for response_backpressure, so exclude as direct target.
    skip=('dr',) if target=='response_backpressure' else ()
    diffs={name:first_diff(base,out/name/'stdout.log',skip if name=='experiment' else ()) for name in ['experiment','unrelated','idempotent']}
    controls_ok=diffs['unrelated'] is None and diffs['idempotent'] is None
    marker_ok=True
    for name in ['experiment','unrelated','idempotent']:
        text=(out/name/'stdout.log').read_text(errors='replace')
        if 'CONTROL|phase=force' not in text or 'CONTROL|phase=release' not in text:marker_ok=False
    runs_completed=all(v['exit_code']==0 and not v['timed_out'] for v in receipts.values())
    result={'status':'PASS' if runs_completed and controls_ok and marker_ok and diffs['experiment'] is not None else 'FAIL',
      'all_simulator_runs_completed':runs_completed,
      'control_comparator':'matched_sampled_cycle_not_formal_causality',
      'prediction_timing':'submitted_before_execution',
      'prediction_judged_by_expert':False,
      'negative_controls_match':controls_ok,'force_release_markers':marker_ok,
      'first_non_target_difference':diffs['experiment'],
      'other_differences':diffs,'independent_oracles':{k:v['oracle_status'] for k,v in receipts.items()},
      'oracle_observations':{k:{'status':v['oracle_status'],'first_issues':v['checks']['issues'][:3],'total_issue_count':v['checks']['total_issue_count']} for k,v in receipts.items()},
      'actual_runs':4}
    (out/'receipt.json').write_text(json.dumps(result,indent=2)+'\n')
    return result

if __name__=='__main__':
    p=argparse.ArgumentParser()
    p.add_argument('--xezim',required=True)
    p.add_argument('--source',required=True)
    p.add_argument('--stimuli',required=True)
    p.add_argument('--out',required=True)
    p.add_argument('--target',choices=TARGETS,required=True)
    p.add_argument('--at-cycle',type=int,required=True)
    p.add_argument('--duration',type=int,required=True)
    p.add_argument('--prediction',required=True)
    a=p.parse_args()
    r=execute(a.xezim,a.source,a.stimuli,a.out,a.target,a.at_cycle,a.duration,a.prediction)
    print(json.dumps(r,indent=2))
    sys.exit(0 if r['status']=='PASS' else 1)
