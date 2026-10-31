#!/usr/bin/env python3
"""Validate a completed private design and write a create-once preregistration.

Does not freeze or generate instances. --review-plan is a plan, not approval.
"""
import argparse,json
from pathlib import Path
from freeze import ROOT
from primary_contract import SCHEMA,digest,load,inside,validate_taxonomy,validate_preregistration

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--taxonomy',required=True);p.add_argument('--review-plan',required=True);a=p.parse_args();path=inside(ROOT,a.taxonomy);taxonomy=load(path);inputs=validate_taxonomy(ROOT,taxonomy)
    protocol={'schema':SCHEMA,'status':'FINAL_PROTOCOL','author_id':taxonomy['author_id'],'model':'gpt-6.1-sol','reasoning_effort':'medium','analysis_seed':71032026,'bootstrap_resamples':10000,'allocation':{'faults':12,'clean_controls':2,'modes':list('ABCD'),'repeats':3,'sessions':168},'limits':{'tool_calls':12,'evidence_bytes':131072,'trial_seconds':1200},'identifiability_policy':taxonomy['identifiability_policy'],'independent_review_plan':a.review_plan,'prior_family_exclusion_policy':'No exposed named/temporal pilot families and no prior DVCon study cases; independent human semantic novelty review of every generated case is mandatory. Hash/token checks alone do not prove semantic novelty.'}
    for key,name in [('rubric','protocol/STAGE1_RUBRIC.md'),('taxonomy',a.taxonomy),('holdout_generator','tools/generate_holdouts.py')]:protocol[key]={'path':name,'sha256':digest(inside(ROOT,name))}
    protocol['protected_inputs']={**inputs,**{protocol[k]['path']:protocol[k]['sha256'] for k in ('rubric','taxonomy','holdout_generator')}};validate_preregistration(ROOT,protocol)
    with (ROOT/'config/preregistration.json').open('x') as f:json.dump(protocol,f,indent=2);f.write('\n')
    print('PREREGISTRATION_PREPARED; not frozen, no holdouts generated or subjects launched')
if __name__=='__main__':main()
