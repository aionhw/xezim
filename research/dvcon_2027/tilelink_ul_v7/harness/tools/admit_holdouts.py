#!/usr/bin/env python3
"""Execute all generated clean/case admissions before independent human review.

No model inference, no source changes, no admission approval is produced here.
An all-or-nothing failure remains immutable in this frozen study.
"""
from __future__ import annotations
import argparse,json,subprocess,shutil,time
from pathlib import Path
from freeze import ROOT,verify as verify_freeze
from primary_contract import verify_generated,inside,load,digest,LABELS,concrete_symptom
from generate_holdouts import ref,save
from run_sim import run
from monitor_oracle import evaluate

def public(path):return [line for line in path.read_text().splitlines() if line.startswith(('PIN|','MON|','SIM_END|'))]
def reference(source,stim,out,config):
    out.mkdir(parents=True,exist_ok=False);shutil.copyfile(stim/'stimulus.hex',out/'stimulus.hex');meta=load(stim/'stimulus.json');compiler=ROOT/'.deps/iverilog-installed/bin/iverilog';vvp=ROOT/'.deps/iverilog-installed/bin/vvp';exe=out/'sim.vvp';began=time.monotonic()
    compile_argv=[str(compiler),'-g2012','-s','tb_tlul','-o',str(exe),str(source),*config.get('extra_sources',[]),config['tb']]
    comp=subprocess.run(compile_argv,capture_output=True,text=True,timeout=120);(out/'compile.log').write_text(comp.stdout+comp.stderr)
    argv=[str(vvp),str(exe),'+STIM=stimulus.hex',f'+NREQ={meta["count"]}',f'+SEED={meta["seed"]}','+CTRL_OP=0']
    timed_out=False;code=None;stdout='';stderr=''
    if comp.returncode==0:
        try:
            result=subprocess.run(argv,cwd=out,capture_output=True,text=True,timeout=120);code=result.returncode;stdout=result.stdout;stderr=result.stderr
        except subprocess.TimeoutExpired as exc:
            timed_out=True;stdout=(exc.stdout or b'').decode(errors='replace') if isinstance(exc.stdout,bytes) else (exc.stdout or '');stderr='Reference runtime deadline exceeded'
    (out/'stdout.log').write_text(stdout);(out/'stderr.log').write_text(stderr);oracle=evaluate(stdout,meta);(out/'oracle.json').write_text(json.dumps(oracle,indent=2)+'\n')
    receipt={'status':'PASS' if code==0 and oracle['status']=='PASS' else 'FAIL','mode':'none','exit_code':code,'timed_out':timed_out,'compile_exit':comp.returncode,'source_sha256':digest(source),'binary_sha256':digest(vvp),'compiler_sha256':digest(compiler),'tb_sha256':digest(Path(config['tb'])),'stimulus_sha256':digest(stim/'stimulus.hex'),'extra_source_sha256':{s:digest(Path(s)) for s in config.get('extra_sources',[])},'case_config':config,'oracle_status':oracle['status'],'wall_seconds':time.monotonic()-began,'compile_argv':compile_argv,'argv':argv}
    (out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');return receipt

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--catalog',type=Path,required=True);a=p.parse_args();frozen=verify_freeze(required=True);catalog=verify_generated(ROOT,a.catalog,frozen)
    out=a.catalog.resolve().parent/'admission_execution_001';out.mkdir(exist_ok=False);receipts=[]
    for case in catalog['cases']:
        records=[];errors=[];cdir=out/case['case_id'];cdir.mkdir()
        for variant in ('clean','case'):
            source=inside(ROOT,case['clean_source' if variant=='clean' else 'source']['path']);cfg=load(inside(ROOT,case['clean_case_manifest' if variant=='clean' else 'case_manifest']['path']))
            for label in LABELS:
                stim=inside(ROOT,case['schedules'][label]['hex']['path']).parent;logs={}
                for sim in ('stock','research','reference'):
                    dst=cdir/f'{variant}_{label}_{sim}'
                    try:
                        if sim=='reference':r=reference(source,stim,dst,cfg)
                        else:
                            binary=ROOT/('.deps/xezim/target/release/xezim' if sim=='stock' else '.deps/xezim-observer/target/release/xezim');r=run(str(binary),source,stim,dst,'none',timeout=300,case_config=cfg)
                        oracle=load(dst/'oracle.json');required='PASS' if variant=='clean' or case['kind']=='clean' else 'FAIL';logs[sim]=public(dst/'stdout.log')
                        if r['exit_code']!=0 or r['timed_out'] or oracle['status']!=required:errors.append(f'{variant}/{label}/{sim}: clean/case expectation or runtime failed')
                        if required=='FAIL' and not concrete_symptom(oracle):errors.append(f'{variant}/{label}/{sim}: no concrete functional/monitor symptom')
                        records.append({'variant':variant,'schedule':label,'simulator':sim,'receipt':ref(ROOT,dst/'receipt.json'),'oracle':ref(ROOT,dst/'oracle.json'),'stdout':ref(ROOT,dst/'stdout.log')})
                    except Exception as exc:
                        errors.append(f'{variant}/{label}/{sim}: {type(exc).__name__}: {exc}')
                        if dst.exists():(dst/'host_failure.json').write_text(json.dumps({'error':str(exc),'scope':'Actual admission execution failure, no model inference'},indent=2)+'\n')
                if len(logs)!=3 or not logs['stock']==logs['research']==logs['reference']:errors.append(f'{variant}/{label}: simulator public traces differ or absent')
        receipt={'status':'PASS' if len(records)==18 and not errors else 'FAIL','case_id':case['case_id'],'catalog_sha256':digest(a.catalog),'runs':records,'errors':errors,'independent_human_review':'PENDING'};save(cdir/'case_admission.json',receipt);receipts.append({'case_id':case['case_id'],'status':receipt['status'],'evidence':ref(ROOT,cdir/'case_admission.json')})
    overall={'status':'PASS_AWAITING_REVIEW' if all(r['status']=='PASS' for r in receipts) else 'FAILED_FROZEN_ADMISSION','catalog_sha256':digest(a.catalog),'freeze_sha256':digest(ROOT/'config/freeze_receipt.json'),'cases':receipts,'primary_subjects':0,'review_complete':False}
    save(out/'receipt.json',overall);print(overall['status']);return 0 if overall['status']=='PASS_AWAITING_REVIEW' else 1
if __name__=='__main__':raise SystemExit(main())
