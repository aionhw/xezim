#!/usr/bin/env python3
"""Create and execute the reviewed 168-cell prospective schedule.

--prepare verifies admission and captures case evidence without inference.
--execute starts only unattempted cells, each in a fresh process/thread.
This controller never retries a failed cell or edits a submitted answer.
"""
from __future__ import annotations
import argparse,ast,datetime,json,os,sys,time
from pathlib import Path
from types import SimpleNamespace
from freeze import ROOT,verify as verify_freeze
from primary_contract import verify_admission,inside,load,digest,canonical_hash,SCHEMA
from run_sim import run
from admit_holdouts import public
sys.path.insert(0,str(ROOT/'agents'))
from run_trial import run_subject,validate_baseline
from grade_patch import grade

def make_plan(catalog):
    cases=sorted(catalog['cases'],key=lambda c:c['case_id']);rows=[]
    if len(cases)!=14 or len({c['case_id'] for c in cases})!=14 or sum(c['kind']=='fault' for c in cases)!=12 or sum(c['kind']=='clean' for c in cases)!=2:raise ValueError('exact reviewed14-case allocation required')
    for repeat in (1,2,3):
        for index,case in enumerate(cases):
            offset=(index+repeat-1)%4;order='ABCD'[offset:]+'ABCD'[:offset]
            for mode in order:rows.append({'case_id':case['case_id'],'mode':mode,'repeat':repeat,'trial_id':f"{case['case_id']}_{mode}_{repeat}"})
    return rows

def save(path,obj):
    # Mutable operational status, not trial outcomes. Atomic replacement avoids
    # truncated status; immutable plan and append-only ledger are separate.
    tmp=path.with_suffix('.tmp')
    with tmp.open('w') as stream:json.dump(obj,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
    tmp.replace(path)
def append(path,obj):
    with path.open('a') as stream:stream.write(json.dumps(obj,sort_keys=True)+'\n');stream.flush();os.fsync(stream.fileno())
def state(out,phase,**fields):save(out/'state.json',{'phase':phase,'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),**fields})

def subject_args(case,row,out,catalog,admission):
    return SimpleNamespace(mode=row['mode'],cohort='primary',trial_id=row['trial_id'],case_id=case['case_id'],repeat=row['repeat'],primary_catalog=catalog,primary_admission=admission,baseline=out/'captures'/case['case_id'],fault_source=inside(ROOT,case['source']['path']),case_manifest=inside(ROOT,case['case_manifest']['path']),stimuli=inside(ROOT,case['schedules']['original']['hex']['path']).parent,xezim=ROOT/'.deps/xezim-observer/target/release/xezim',out=out/'subjects'/row['trial_id'],timeout=1200,engineering_external=False)

def prepare(catalog_path,admission_path,out,frozen):
    catalog,admission=verify_admission(ROOT,catalog_path,admission_path,frozen)
    # One campaign per frozen catalog, regardless of selected output directory.
    claim=catalog_path.resolve().parent/'primary_campaign_claim.json'
    with claim.open('x') as stream:
        json.dump({'catalog_sha256':digest(catalog_path),'admission_sha256':digest(admission_path),'out':str(out)},stream);stream.write('\n');stream.flush();os.fsync(stream.fileno())
    claim.chmod(0o400);out.mkdir(parents=True,exist_ok=False)
    plan={'schema':SCHEMA,'cohort':'primary','catalog_path':str(catalog_path.resolve()),'catalog_sha256':digest(catalog_path),'admission_path':str(admission_path.resolve()),'admission_sha256':digest(admission_path),'freeze_sha256':digest(ROOT/'config/freeze_receipt.json'),'rows':make_plan(catalog)}
    with (out/'plan.json').open('x') as stream:json.dump(plan,stream,indent=2);stream.write('\n')
    (out/'plan.json').chmod(0o400);state(out,'PREPARING_EVIDENCE',primary_subjects=0)
    try:
        for case in catalog['cases']:
            args=subject_args(case,{'mode':'D','repeat':1,'trial_id':case['case_id']+'_D_1'},out,catalog_path,admission_path)
            result=run(args.xezim,args.fault_source,args.stimuli,args.baseline,'native',capture_wave=True,case_config=load(args.case_manifest))
            validate_baseline(args)
            evidence=load(inside(ROOT,admission['cases'][case['case_id']]['execution_evidence']['path']))
            original=next(r for r in evidence['runs'] if r['variant']=='case' and r['schedule']=='original' and r['simulator']=='research')
            if public(args.baseline/'stdout.log')!=public(inside(ROOT,original['stdout']['path'])):raise ValueError('native capture changed admitted public trace')
            expected='FAIL' if case['kind']=='fault' else 'PASS'
            if result['exit_code']!=0 or result['oracle_status']!=expected:raise ValueError('capture no longer matches admitted case')
        state(out,'PREPARED',primary_subjects=0,plan_sha256=digest(out/'plan.json'))
    except Exception as exc:
        state(out,'HALTED_PREPARATION',primary_subjects=0,error=type(exc).__name__+': '+str(exc));raise

def continuation_allowed(record):
    if record.get('status')=='COMPLETED':return True
    # Expected subject failures preserve the denominator. Exposure/identity,
    # transport, blinding and native/checker failures halt the whole campaign.
    message=record.get('error','')
    prefix='AppServerError: capability/model violation: '
    if message.startswith(prefix):
        try:violations=ast.literal_eval(message[len(prefix):])
        except (ValueError,SyntaxError):return False
        return isinstance(violations,list) and bool(violations) and all(isinstance(v,str) for v in violations) and set(violations)=={'case tool action limit exceeded'}
    return message.startswith(('ValueError: Stage I schema incomplete','ValueError: Stage I missing/invalid issued evidence','ValueError: required mode evidence unused','ValueError: D native query missing','JSONDecodeError:','ValueError: trial deadline exceeded'))

def execute(out,frozen):
    plan=load(out/'plan.json');status=load(out/'state.json')
    if status['phase'] not in ('PREPARED','RUNNING'):raise ValueError('campaign halted or complete; do not restart it under a new output directory')
    if status.get('plan_sha256')!=digest(out/'plan.json') or plan['freeze_sha256']!=digest(ROOT/'config/freeze_receipt.json') or digest(plan['catalog_path'])!=plan['catalog_sha256'] or digest(plan['admission_path'])!=plan['admission_sha256']:raise ValueError('plan/freeze/catalog/admission identity drift')
    catalog,_=verify_admission(ROOT,Path(plan['catalog_path']),Path(plan['admission_path']),frozen)
    if plan['rows']!=make_plan(catalog):raise ValueError('schedule differs from frozen allocation rule')
    ledger=out/'ledger.jsonl';completed=[json.loads(line) for line in ledger.read_text().splitlines()] if ledger.exists() else []
    if len(completed)>168 or [r['trial_id'] for r in completed]!=[r['trial_id'] for r in plan['rows'][:len(completed)]]:raise ValueError('ledger is not an append-only plan prefix')
    if completed and completed[-1].get('halt_campaign'):raise ValueError('campaign previously halted; do not continue')
    cases={c['case_id']:c for c in catalog['cases']};codex=ROOT/load(ROOT/'config/model.json')['codex_cli_path']
    # Exclusive controller lock prevents concurrent inference. If interrupted,
    # retain the lock and inspect the unfinished cell; never automatically retry.
    lock=out/'execution_claim.json'
    with lock.open('x') as stream:json.dump({'pid':os.getpid(),'plan_sha256':digest(out/'plan.json')},stream)
    state(out,'RUNNING',primary_subjects=len(completed),plan_sha256=digest(out/'plan.json'))
    try:
        for row in plan['rows'][len(completed):]:
            args=subject_args(cases[row['case_id']],row,out,Path(plan['catalog_path']),Path(plan['admission_path']))
            cell_began=time.monotonic()
            try:
                rec=run_subject(args,codex=str(codex));subject_finished=time.monotonic()
                graded=grade(args.out/'trial.json',args.xezim,args.fault_source,out/'grades'/row['trial_id'])
                usage=rec.get('cache_observations') or []
                total=usage[-1].get('tokenUsage',{}).get('total',{}) if usage else {}
                result={**row,'trial_status':rec['status'],'runtime_grade':graded['status'],'error':rec.get('error'),'halt_campaign':not continuation_allowed(rec),'reported_trial_wall_seconds':rec['elapsed_seconds'],'host_subject_wall_seconds':subject_finished-cell_began,'grading_wall_seconds':time.monotonic()-subject_finished,'input_tokens':total.get('inputTokens'),'output_tokens':total.get('outputTokens'),'cached_input_tokens':total.get('cachedInputTokens')}

            except Exception as exc:
                result={**row,'trial_status':'HOST_FAILURE_OR_REFUSAL','runtime_grade':'NOT_RUN','error':type(exc).__name__+': '+str(exc),'halt_campaign':True}
            result['total_cell_wall_seconds']=time.monotonic()-cell_began
            append(ledger,result);completed.append(result)
            state(out,'HALTED' if result['halt_campaign'] else 'RUNNING',primary_subjects=len(completed),plan_sha256=digest(out/'plan.json'),last_result=result)
            if result['halt_campaign']:return 1
        state(out,'FINISHED_AWAITING_BLINDED_REVIEW',primary_subjects=168,plan_sha256=digest(out/'plan.json'));return 0
    finally:
        # Normal controller completion releases only the operational lock. A
        # hard interruption leaves it in place; subject claims always persist.
        lock.unlink(missing_ok=True)

def main():
    p=argparse.ArgumentParser(description=__doc__);choice=p.add_mutually_exclusive_group(required=True);choice.add_argument('--prepare',action='store_true');choice.add_argument('--execute',action='store_true');p.add_argument('--catalog',type=Path);p.add_argument('--admission',type=Path);p.add_argument('--out',type=Path,required=True);a=p.parse_args();out=a.out.resolve()
    if not out.is_relative_to(ROOT/'results'):raise ValueError('campaign must remain in isolated results directory')
    frozen=verify_freeze(required=True)
    if a.prepare:
        if not a.catalog or not a.admission:p.error('--prepare requires --catalog and --admission')
        prepare(a.catalog,a.admission,out,frozen);return 0
    return execute(out,frozen)
if __name__=='__main__':raise SystemExit(main())
