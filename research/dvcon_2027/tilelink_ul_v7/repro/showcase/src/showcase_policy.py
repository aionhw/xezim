"""Explicit user-authorized showcase scope, with automated gates retained.

This module does not create human review or authorize a primary comparison.
"""
from pathlib import Path
import json,hashlib,os,re,sys
ROOT=Path(__file__).resolve().parents[3];OVERLAY=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools'))
from primary_contract import load,digest,inside,verify_generated,checked_file,LABELS,concrete_symptom
from monitor_oracle import evaluate
CATALOG=ROOT/'private/holdouts_primary_001/catalog.json'

def overlay_inventory():return {str(p.relative_to(OVERLAY)):digest(p) for p in sorted((OVERLAY/'src').glob('*.py'))}
def verify_overlay():
 manifest=load(OVERLAY/'OVERLAY_SEAL.json')
 if manifest['source_sha256']!=overlay_inventory() or manifest['scope_amendment_sha256']!=digest(OVERLAY/'USER_SCOPE_AMENDMENT.json'):raise ValueError('showcase overlay changed after seal')
 authority=load(OVERLAY/'USER_SCOPE_AMENDMENT.json')
 if authority.get('status')!='USER_AUTHORIZED_SHOWCASE_ONLY' or authority.get('human_review')!='WAIVED_BY_USER_FOR_SHOWCASES; no reviewer identity or approval invented':raise ValueError('explicit showcase scope missing')
 return authority

def verify_showcase_inputs(frozen):
 authority=verify_overlay()
 if not frozen or digest(ROOT/'config/freeze_receipt.json')!=authority['base_freeze_sha256'] or digest(CATALOG)!=authority['catalog_sha256']:raise ValueError('base freeze/catalog identity changed')
 catalog=verify_generated(ROOT,CATALOG,frozen);automatic=CATALOG.parent/'admission_execution_001/receipt.json';audit=ROOT/'results/frozen_admission_audit_001/receipt_private.json'
 if digest(automatic)!=authority['automated_admission_sha256'] or digest(audit)!=authority['admission_audit_sha256']:raise ValueError('automated admission/audit identity mismatch')
 admission=load(automatic)
 if admission['status']!='PASS_AWAITING_REVIEW' or admission['catalog_sha256']!=digest(CATALOG) or load(audit)['status']!='PASS':raise ValueError('automated evidence is not qualified')
 entries={row['case_id']:row for row in admission['cases']}
 if len(entries)!=14 or set(entries)!={c['case_id'] for c in catalog['cases']}:raise ValueError('showcase requires all14 admitted cases')
 for case in catalog['cases']:
  row=entries[case['case_id']];evidence=load(checked_file(ROOT,row['evidence']))
  if row['status']!=evidence['status'] or evidence['status']!='PASS' or evidence['catalog_sha256']!=digest(CATALOG):raise ValueError('case automated admission mismatch')
  runs=evidence['runs'];expected={(v,s,t) for v in ('clean','case') for s in LABELS for t in ('stock','research','reference')}
  if len(runs)!=18 or {(r['variant'],r['schedule'],r['simulator']) for r in runs}!=expected:raise ValueError('incomplete automated admission')
  traces={}
  for r in runs:
   receipt=load(checked_file(ROOT,r['receipt']));log=checked_file(ROOT,r['stdout']);oracle=load(checked_file(ROOT,r['oracle']));meta=load(inside(ROOT,case['schedules'][r['schedule']]['metadata']['path']));required='PASS' if r['variant']=='clean' or case['kind']=='clean' else 'FAIL'
   if evaluate(log.read_text(),meta)!=oracle or oracle['status']!=required or receipt['oracle_status']!=required:raise ValueError('admission checker inconsistency')
   source=case['clean_source' if r['variant']=='clean' else 'source']
   if receipt['exit_code']!=0 or receipt['timed_out'] or receipt['source_sha256']!=source['sha256'] or receipt['stimulus_sha256']!=case['schedules'][r['schedule']]['hex']['sha256']:raise ValueError('admission run identity/runtime failed')
   binary={'stock':'.deps/xezim/target/release/xezim','research':'.deps/xezim-observer/target/release/xezim','reference':'.deps/iverilog-installed/bin/vvp'}[r['simulator']]
   if receipt['binary_sha256']!=frozen['artifacts'][binary] or (required=='FAIL' and not concrete_symptom(oracle)):raise ValueError('unqualified admission binary/symptom')
   traces[r['variant'],r['schedule'],r['simulator']]=[s for s in log.read_text().splitlines() if s.startswith(('PIN|','MON|','SIM_END|'))]
  for variant in ('clean','case'):
   for label in LABELS:
    if not traces[variant,label,'stock']==traces[variant,label,'research']==traces[variant,label,'reference']:raise ValueError('cross-simulator disagreement')
 return catalog

def authorize_showcase(args,frozen):
 catalog=verify_showcase_inputs(frozen)
 if args.cohort!='showcase' or args.repeat!=1 or args.timeout!=1200 or args.engineering_external:raise ValueError('showcase mode/deadline/tier mismatch')
 matches=[c for c in catalog['cases'] if c['case_id']==args.case_id]
 if len(matches)!=1:raise ValueError('case not admitted')
 case=matches[0];trial_id=f"showcase_{case['case_id']}_{args.mode}_1"
 if args.mode not in 'ABCD' or args.trial_id!=trial_id or Path(args.out).resolve()!=OVERLAY/'subjects'/trial_id:raise ValueError('showcase cell/output mismatch')
 if Path(args.fault_source).resolve()!=inside(ROOT,case['source']['path']) or not args.case_manifest or Path(args.case_manifest).resolve()!=inside(ROOT,case['case_manifest']['path']) or Path(args.stimuli).resolve()!=inside(ROOT,case['schedules']['original']['hex']['path']).parent:raise ValueError('showcase source/config/stimulus mismatch')
 if digest(args.xezim)!=frozen['artifacts']['.deps/xezim-observer/target/release/xezim']:raise ValueError('showcase simulator mismatch')
 return {'scope':'SHOWCASE_ONLY','human_review':'WAIVED_BY_USER_FOR_SHOWCASES','case_id':case['case_id'],'repeat':1,'catalog_path':str(CATALOG),'catalog_sha256':digest(CATALOG),'base_freeze_sha256':digest(ROOT/'config/freeze_receipt.json'),'scope_amendment_sha256':digest(OVERLAY/'USER_SCOPE_AMENDMENT.json'),'overlay_seal_sha256':digest(OVERLAY/'OVERLAY_SEAL.json')}

def reserve_showcase(authorization,trial_id):
 if not re.fullmatch(r'showcase_case_[0-9a-f]{16}_[ABCD]_1',trial_id):raise ValueError('invalid showcase cell')
 claims=OVERLAY/'claims';claims.mkdir(exist_ok=True)
 with (claims/(trial_id+'.json')).open('x') as stream:json.dump(authorization,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
 (claims/(trial_id+'.json')).chmod(0o400)
