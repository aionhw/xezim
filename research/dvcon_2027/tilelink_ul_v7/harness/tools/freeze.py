#!/usr/bin/env python3
"""Create-once pre-holdout freeze, bound to actual qualified artifacts."""
import argparse,datetime,hashlib,json,sys
from pathlib import Path
from primary_contract import validate_preregistration
ROOT=Path(__file__).resolve().parents[1]
INCLUDED=['rtl','tools','agents','protocol','config','tests']
SKIP={'binary_sha256.txt','freeze_receipt.json','STATUS.json'}
FREEZE=ROOT/'config/freeze_receipt.json'
def digest(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def inventory():
 files={}
 for folder in INCLUDED:
  for p in (ROOT/folder).rglob('*'):
   if p.is_file() and p.name not in SKIP and '__pycache__' not in p.parts and p.suffix!='.pyc':files[str(p.relative_to(ROOT))]=digest(p)
 for name in ('PLAN.md','README.md','AGENT_HANDOFF.md','PROMPT_FOR_CODEX_PRO.md'):
  files[name]=digest(ROOT/name)
 return dict(sorted(files.items()))
def artifacts():
 conf=json.loads((ROOT/'config/model.json').read_text())
 names=['.deps/xezim/target/release/xezim','.deps/xezim/Cargo.lock','.deps/xezim-observer/target/release/xezim','.deps/xezim-observer/Cargo.lock','.deps/iverilog-installed/bin/iverilog','.deps/iverilog-installed/bin/vvp',conf['codex_cli_path'],'.deps/codex-protocol-source/codex-rs/Cargo.lock','results/codex_strict_tool_policy.patch','results/case_permissions_qualification.json','results/case_tool_allowlist_qualification.json','results/native_qualification.json','results/clean_admission/receipt.json','results/control_qualification_001/receipt.json','results/temporal_control_qualification_001/receipt.json','results/strict_pilot_capture_001/receipt.json','results/strict_pilot_schedule_001.jsonl']
 names.extend(['results/prospective_clean_002/receipt.json','results/prospective_controls_002/receipt.json','results/prospective_canaries_002/receipt.json','results/tests_replacement_pre_freeze_002.log'])
 names.extend(['private/pilots_exposed_001/pending_wrong_retirement.sv','private/pilots_exposed_001/wrong_response_word.sv','results/budget_briefing_qualification_001/receipt.json'])
 return {name:digest(ROOT/name) for name in names}
def verify(required=False):
 if not FREEZE.is_file():
  if required:raise ValueError('primary cohort requires a pre-holdout freeze')
  return None
 previous=json.loads(FREEZE.read_text())
 if previous.get('files')!=inventory() or previous.get('artifacts')!=artifacts():raise ValueError('freeze mismatch; do not run or score primary subjects')
 protected=validate_preregistration(ROOT,json.loads((ROOT/'config/preregistration.json').read_text()))
 if previous.get('protected_inputs')!=protected:raise ValueError('private design inputs differ from freeze')
 return previous

def main():
 p=argparse.ArgumentParser();p.add_argument('--create',action='store_true');p.add_argument('--verify',action='store_true');a=p.parse_args()
 if a.create:
  if FREEZE.exists():raise ValueError('freeze exists; use a new study for protocol changes')
  prereg=ROOT/'config/preregistration.json'
  if not prereg.is_file():raise ValueError('preregistration, rubric, independent taxonomy and holdout generator must be finalized first')
  protocol=json.loads(prereg.read_text())
  protected=validate_preregistration(ROOT,protocol)
  ledger=[json.loads(s) for s in (ROOT/'results/strict_pilot_schedule_001.jsonl').read_text().splitlines()]
  expected={(f'{family}_{i:02d}',m) for family,n in [('named',5),('temporal',8)] for i in range(1,n+1) for m in 'ABCD'}
  if len(ledger)!=52 or {(r['case'],r['mode']) for r in ledger}!=expected:raise ValueError('all strict pilot sessions must be preserved before freeze')
  for name in ('case_permissions_qualification','case_tool_allowlist_qualification','native_qualification'):
   if json.loads((ROOT/'results'/(name+'.json')).read_text()).get('status')!='PASS':raise ValueError('qualification failed: '+name)
  if (ROOT/'private/primary_generation_claim.json').exists():raise ValueError('primary generation already claimed before freeze')
  if any((ROOT/'private').glob('holdout*')):raise ValueError('holdout artifacts already exist before freeze')
  sys.path.insert(0,str(ROOT/'agents'));from codex_app_server import CodexAppServer
  conf=json.loads((ROOT/'config/model.json').read_text())
  with CodexAppServer(codex=str(ROOT/conf['codex_cli_path'])) as app:
   app.initialize();preflight=app.preflight(conf['model'],conf['reasoning_effort'])
  receipt={'status':'FROZEN','utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'timestamp_scope':'local host clock; readonly file prevents accidental edits, not an external timestamp attestation','files':inventory(),'artifacts':artifacts(),'protected_inputs':protected,'model_preflight':preflight,'holdouts_generated':False}
  with FREEZE.open('x') as f:
   json.dump(receipt,f,indent=2);f.write('\n');f.flush()
   import os
   os.fsync(f.fileno())
  FREEZE.chmod(0o444);print('FROZEN',len(receipt['files']),'files')
 elif a.verify:verify(required=True);print('PASS: code and qualified artifact freeze match')
 else:p.error('specify --create or --verify')
if __name__=='__main__':main()
