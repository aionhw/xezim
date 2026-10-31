#!/usr/bin/env python3
"""Prepare an explicitly UNAPPROVED human review form after actual admission.

No person, expertise, independence or review decision is filled automatically.
"""
import argparse,json
from pathlib import Path
from freeze import ROOT,verify as verify_freeze
from primary_contract import verify_generated,load,digest
from generate_holdouts import save

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--catalog',type=Path,required=True);a=p.parse_args();frozen=verify_freeze(required=True);catalog=verify_generated(ROOT,a.catalog,frozen);base=a.catalog.resolve().parent;automatic=load(base/'admission_execution_001/receipt.json')
 if automatic.get('status')!='PASS_AWAITING_REVIEW' or automatic.get('catalog_sha256')!=digest(a.catalog):raise ValueError('all frozen automated admissions must pass before independent approval')
 evidence={r['case_id']:r['evidence'] for r in automatic['cases']}
 form={'schema':'TLUL_PRIMARY_V1','status':'DRAFT_NOT_APPROVED','catalog_sha256':digest(a.catalog),'review':{'origin':None,'reviewer_id':None,'expertise':None,'utc':None,'independent_of_construction':None},'instructions':'An actual independent human must review novelty, identifiability, clean/case executions and checker separation for every case. Only after a real favorable review may they supply origin=human, real review fields, all admitted/novelty decisions and status=APPROVED. Frozen identifiability_class is copied for review and must not be broadened. Do not fabricate identity or decisions.','cases':{c['case_id']:{'admitted':None,'novelty_review_passed':None,'identifiability_class':c['identifiability'],'reason':None,'execution_evidence':evidence[c['case_id']]} for c in catalog['cases']}}
 save(base/'admission_review_form.json',form);print('DRAFT_NOT_APPROVED: private14-case review form created; zero primary subjects authorized')
if __name__=='__main__':main()
