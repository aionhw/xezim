#!/usr/bin/env python3
"""Execute56 descriptive showcase cells; human review explicitly user-waived."""
from pathlib import Path
import datetime,json,os,subprocess,sys,time
ROOT=Path(__file__).resolve().parents[3];OVERLAY=Path(__file__).resolve().parents[1];SRC=OVERLAY/'src'
sys.path.insert(0,str(ROOT/'tools'));sys.path.insert(0,str(ROOT/'agents'));sys.path.insert(0,str(SRC))
from freeze import verify as verify_freeze
from primary_contract import load,digest,inside
from run_primary_schedule import make_plan,continuation_allowed
from run_sim import run
from admit_holdouts import public
from showcase_trial import validate_baseline
from showcase_policy import verify_showcase_inputs,verify_overlay,CATALOG
from types import SimpleNamespace

def save(path,obj):
 tmp=path.with_suffix('.tmp')
 with tmp.open('w') as stream:json.dump(obj,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
 tmp.replace(path)
def status(phase,**fields):save(OVERLAY/'state.json',{'phase':phase,'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'scope':'SHOWCASE_ONLY_HUMAN_REVIEW_WAIVED',**fields})
def append(obj):
 with (OVERLAY/'ledger.jsonl').open('a') as stream:stream.write(json.dumps(obj,sort_keys=True)+'\n');stream.flush();os.fsync(stream.fileno())

def main():
 verify_overlay();frozen=verify_freeze(required=True);catalog=verify_showcase_inputs(frozen)
 if (OVERLAY/'run_claim.json').exists():raise ValueError('showcase already attempted; inspect status, do not replay outcomes')
 with (OVERLAY/'run_claim.json').open('x') as stream:json.dump({'pid':os.getpid(),'overlay_seal_sha256':digest(OVERLAY/'OVERLAY_SEAL.json')},stream)
 rows=[{**r,'trial_id':'showcase_'+r['trial_id']} for r in make_plan(catalog) if r['repeat']==1]
 with (OVERLAY/'plan.json').open('x') as stream:json.dump({'scope':'SHOWCASE_ONLY','human_review':'WAIVED_BY_USER','repeats':1,'scheduled_subjects':56,'catalog_sha256':digest(CATALOG),'rows':rows},stream,indent=2);stream.write('\n')
 (OVERLAY/'plan.json').chmod(0o400)
 # Retire this exposed catalog from the unused primary controller. This is a
 # cancellation marker, explicitly zero actual primary subjects, not review.
 with (CATALOG.parent/'primary_campaign_claim.json').open('x') as stream:json.dump({'status':'RETIRED_FOR_SHOWCASES_NO_PRIMARY_EXECUTION','user_instruction':'this is for showcases. skip human review','showcase_overlay':str(OVERLAY),'primary_subjects':0},stream,indent=2);stream.write('\n')
 (CATALOG.parent/'primary_campaign_claim.json').chmod(0o400)
 status('PREPARING_CASE_CAPTURES',recorded_subjects=0);cases={c['case_id']:c for c in catalog['cases']};binary=ROOT/'.deps/xezim-observer/target/release/xezim';codex=ROOT/load(ROOT/'config/model.json')['codex_cli_path']
 try:
  for c in catalog['cases']:
   dst=OVERLAY/'captures'/c['case_id'];src=inside(ROOT,c['source']['path']);stim=inside(ROOT,c['schedules']['original']['hex']['path']).parent;cfgpath=inside(ROOT,c['case_manifest']['path']);result=run(binary,src,stim,dst,'native',capture_wave=True,case_config=load(cfgpath));args=SimpleNamespace(mode='D',baseline=dst,fault_source=src,stimuli=stim,xezim=binary,case_manifest=cfgpath,engineering_external=False);validate_baseline(args)
   evidence=load(CATALOG.parent/'admission_execution_001'/c['case_id']/'case_admission.json');original=next(r for r in evidence['runs'] if r['variant']=='case' and r['schedule']=='original' and r['simulator']=='research')
   if public(dst/'stdout.log')!=public(inside(ROOT,original['stdout']['path'])) or result['oracle_status']!=('FAIL' if c['kind']=='fault' else 'PASS'):raise ValueError('showcase capture differs from admitted execution')
 except Exception as exc:
  status('HALTED_CAPTURE_QUALIFICATION',recorded_subjects=0,error=type(exc).__name__+': '+str(exc));raise
 for ordinal,row in enumerate(rows):
  case=cases[row['case_id']];out=OVERLAY/'subjects'/row['trial_id'];gradeout=OVERLAY/'grades'/row['trial_id'];logs=OVERLAY/'controller_logs';logs.mkdir(exist_ok=True);start=time.monotonic();status('RUNNING',recorded_subjects=ordinal,current_cell=row['trial_id'])
  argv=[sys.executable,str(SRC/'showcase_trial.py'),'--cohort','showcase','--mode',row['mode'],'--case-id',row['case_id'],'--repeat','1','--trial-id',row['trial_id'],'--baseline',str(OVERLAY/'captures'/row['case_id']),'--fault-source',str(inside(ROOT,case['source']['path'])),'--case-manifest',str(inside(ROOT,case['case_manifest']['path'])),'--stimuli',str(inside(ROOT,case['schedules']['original']['hex']['path']).parent),'--xezim',str(binary),'--codex-bin',str(codex),'--out',str(out)]
  try:
   with (logs/(row['trial_id']+'_subject.log')).open('w') as stream:
    proc=subprocess.Popen(argv,stdout=stream,stderr=subprocess.STDOUT,cwd=ROOT);python_pid=proc.pid;subject_exit=proc.wait(timeout=1500)
   rec=load(out/'trial.json');subject_end=time.monotonic()
   grade_argv=[sys.executable,str(SRC/'showcase_grade.py'),'--trial',str(out/'trial.json'),'--xezim',str(binary),'--fault-source',str(inside(ROOT,case['source']['path'])),'--out',str(gradeout)]
   with (logs/(row['trial_id']+'_grade.log')).open('w') as stream:grading=subprocess.run(grade_argv,stdout=stream,stderr=subprocess.STDOUT,cwd=ROOT,timeout=900)
   graded=load(gradeout/'grade.json');usage=rec.get('cache_observations') or [];total=usage[-1].get('tokenUsage',{}).get('total',{}) if usage else {};result={**row,'trial_status':rec['status'],'runtime_grade':graded['status'],'error':rec.get('error'),'halt_campaign':not continuation_allowed(rec),'python_process_id':python_pid,'codex_process_id':rec.get('process_id'),'thread_id':rec.get('thread_id'),'reported_trial_wall_seconds':rec['elapsed_seconds'],'host_subject_wall_seconds':subject_end-start,'grading_wall_seconds':time.monotonic()-subject_end,'input_tokens':total.get('inputTokens'),'output_tokens':total.get('outputTokens'),'cached_input_tokens':total.get('cachedInputTokens'),'subject_exit':subject_exit,'grade_exit':grading.returncode}
  except Exception as exc:
   if 'proc' in locals() and proc.poll() is None:proc.terminate();proc.wait(timeout=30)
   result={**row,'trial_status':'HOST_FAILURE_OR_REFUSAL','runtime_grade':'NOT_RUN','error':type(exc).__name__+': '+str(exc),'halt_campaign':True}
  result['total_cell_wall_seconds']=time.monotonic()-start;append(result);print(json.dumps(result),flush=True)
  if result['halt_campaign']:
   status('HALTED',recorded_subjects=ordinal+1,last_result=result);return 1
 status('FINISHED_SHOWCASES_NO_EXPERT_SCORES',recorded_subjects=56,primary_subjects=0);return 0
if __name__=='__main__':raise SystemExit(main())
