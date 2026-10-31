#!/usr/bin/env python3
"""Independent one-edit grader: rejects bypasses, runs three fresh legal schedules.

The submitted patch is preserved unchanged. Grading never edits the model answer.
"""
import argparse, json, re, shutil, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools'))
sys.path.insert(0,str(ROOT/'agents'))
from generate_stimulus import write
from run_sim import run
from freeze import verify as verify_freeze
from primary_contract import verify_admission, digest, inside, LABELS
from showcase_policy import verify_showcase_inputs
ROOT=Path(__file__).resolve().parents[3]
BANNED=(r'\$finish',r'\$fatal',r'\$display',r'//.*(?:checker|oracle|skip|bypass)',
        r'\$test\$plusargs',r'\$value\$plusargs',r'`',
        r'\b(?:import|export|bind|module|endmodule|program|interface|package|force|release|tb_tlul)\b')
SAFE_SYSTEM_FUNCTIONS={'bits','clog2','signed','unsigned','countones','onehot','onehot0','isunknown'}
def forbidden_construct(text):
    return (any(re.search(p,text,re.IGNORECASE) for p in BANNED) or
            any(name.lower() not in SAFE_SYSTEM_FUNCTIONS for name in re.findall(r'\$([a-zA-Z_][a-zA-Z_0-9]*)',text)))

def grade(trial,xezim,fault_source,out,seed=7103):
    data=json.loads(Path(trial).read_text())
    frozen=verify_freeze(required=data.get('cohort') in ('primary','showcase'))
    primary_schedules=None
    if data.get('cohort')=='primary':
        authorization=data.get('primary_authorization') or {}
        if not all(authorization.get(k) for k in ('catalog_path','admission_path','catalog_sha256','admission_sha256','case_id','freeze_sha256')):
            raise ValueError('primary grade requires original reviewed-case authorization')
        if digest(authorization['catalog_path'])!=authorization['catalog_sha256'] or digest(authorization['admission_path'])!=authorization['admission_sha256'] or digest(ROOT/'config/freeze_receipt.json')!=authorization['freeze_sha256']:
            raise ValueError('primary admission identity changed since subject execution')
        catalog,_=verify_admission(ROOT,authorization['catalog_path'],authorization['admission_path'],frozen)
        cases=[c for c in catalog['cases'] if c['case_id']==authorization['case_id']]
        if len(cases)!=1 or digest(fault_source)!=cases[0]['source']['sha256']:
            raise ValueError('primary grade source does not match reviewed case')
        case=cases[0]
        if data.get('case_config')!=json.loads(inside(ROOT,case['case_manifest']['path']).read_text()):
            raise ValueError('primary grade configuration differs from reviewed case')
        if digest(xezim)!=frozen['artifacts']['.deps/xezim-observer/target/release/xezim']:
            raise ValueError('primary grade simulator differs from freeze')
        if data.get('patch_target')!=data['case_config'].get('patch_target'):
            raise ValueError('primary patch target differs from reviewed case')
        primary_schedules={label:inside(ROOT,case['schedules'][label]['hex']['path']).parent for label in LABELS}
    if data.get('cohort')=='showcase':
        authorization=data.get('showcase_authorization') or {}
        catalog=verify_showcase_inputs(frozen)
        if authorization.get('catalog_sha256')!=digest(authorization.get('catalog_path','')) or authorization.get('base_freeze_sha256')!=digest(ROOT/'config/freeze_receipt.json'):
            raise ValueError('showcase authorization identity mismatch')
        matches=[c for c in catalog['cases'] if c['case_id']==authorization.get('case_id')]
        if len(matches)!=1 or digest(fault_source)!=matches[0]['source']['sha256']:
            raise ValueError('showcase grade source mismatch')
        case=matches[0]
        if data.get('case_config')!=json.loads(inside(ROOT,case['case_manifest']['path']).read_text()) or data.get('patch_target')!=data['case_config'].get('patch_target'):
            raise ValueError('showcase target/configuration mismatch')
        if digest(xezim)!=frozen['artifacts']['.deps/xezim-observer/target/release/xezim']:
            raise ValueError('showcase grade simulator mismatch')
        primary_schedules={label:inside(ROOT,case['schedules'][label]['hex']['path']).parent for label in LABELS}
    trial=Path(trial);out=Path(out).resolve()
    if out.exists():raise FileExistsError('cannot overwrite grader output')
    out.mkdir(parents=True)
    data=json.loads(trial.read_text());submission=data.get('stages',{}).get('II',{}).get('submission') or {}
    case_config=data.get('case_config') or {}
    target=data.get('patch_target','rtl/tlul_mem_endpoint.sv')
    result={'status':'UNQUALIFIED','trial_status':data.get('status'),
            'stage1_sha256':data.get('stages',{}).get('I',{}).get('sha256'),
            'patch_policy_pass':False,'run_records':[]}
    (out/'submitted_patch.json').write_text(json.dumps(submission,indent=2)+'\n')
    if data.get('status')!='COMPLETED':
        result['reason']='noncompleted_trial'
    elif target not in ('rtl/tlul_mem_endpoint.sv','rtl/tlul_components.sv'):
        result['reason']='protected_or_unknown_path'
    elif all(key in submission and submission[key] is None for key in ('path','old','new')):
        result['patch_policy_pass']=True
        original=Path(data['stimuli_path'])
        for label,case_seed in [('original',seed),('unseen',seed+4711),('adversarial',seed+9917)]:
            stim=out/f'stim_{label}'
            if primary_schedules:shutil.copytree(primary_schedules[label],stim)
            elif label=='original':shutil.copytree(original,stim)
            else:
                write(case_seed,96,stim)
                if json.loads((original/'stimulus.json').read_text()).get('check_monitor_claims'):
                    m=json.loads((stim/'stimulus.json').read_text());m['check_monitor_claims']=True
                    (stim/'stimulus.json').write_text(json.dumps(m,indent=2)+'\n')
            path=out/f'run_{label}'
            r=run(xezim,fault_source,stim,path,'none',timeout=300,case_config=case_config)
            result['run_records'].append({'label':label,'status':r['status'],
                'oracle':r['oracle_status'],'receipt':str(path/'receipt.json')})
        result['status']=('VERIFIED_CLEAN' if all(r['status']=='PASS' for r in result['run_records'])
                          else 'FAILED_CLEAN_CONFIRMATION')
    elif submission.get('path')!=target:
        result['reason']='protected_or_unknown_path'
    else:
        old=submission.get('old','');new=submission.get('new','')
        text=Path(fault_source).read_text()
        if not isinstance(old,str) or not isinstance(new,str) or len(old)<5 or old==new:
            result['reason']='invalid_edit'
        elif text.count(old)!=1:
            result['reason']='edit_anchor_not_unique'
        elif forbidden_construct(new):
            result['reason']='bypass_pattern'
        elif abs(new.count('\n')-old.count('\n'))>12 or len(new)>4000:
            result['reason']='patch_too_large'
        else:
            result['patch_policy_pass']=True
            patched=out/'patched.sv'
            patched.write_text(text.replace(old,new,1))
            good=0
            original=Path(data['stimuli_path']) if data.get('stimuli_path') else None
            for label,case_seed in [('original',seed),('unseen',seed+4711),('adversarial',seed+9917)]:
                stim=out/f'stim_{label}'
                if primary_schedules:shutil.copytree(primary_schedules[label],stim)
                elif label=='original' and original is not None:shutil.copytree(original,stim)
                else:
                    write(case_seed,96,stim)
                    if original is not None and json.loads((original/'stimulus.json').read_text()).get('check_monitor_claims'):
                        m=json.loads((stim/'stimulus.json').read_text());m['check_monitor_claims']=True;(stim/'stimulus.json').write_text(json.dumps(m,indent=2)+'\n')
                path=out/f'run_{label}'
                r=run(xezim,patched,stim,path,'none',timeout=300,case_config=case_config)
                result['run_records'].append({'label':label,'status':r['status'],
                                               'oracle':r['oracle_status'],'receipt':str(path/'receipt.json')})
                if r['status']=='PASS':good+=1
            canary_ok=True
            if case_config.get('role') in ('monitor','scoreboard'):
                family='pending_wrong_retirement' if case_config['role']=='monitor' else 'wrong_response_word'
                canary=dict(case_config);canary['extra_sources']=[str(ROOT/'private/pilots_exposed_001'/(family+'.sv'))]
                c=run(xezim,patched,out/'stim_original',out/'monitor_canary','none',timeout=300,case_config=canary)
                alarm='source_alarm=1' if case_config['role']=='monitor' else 'data_alarm=1'
                canary_ok=c['exit_code']==0 and c['oracle_status']=='FAIL' and alarm in (out/'monitor_canary/stdout.log').read_text()
                result['monitor_canary']={'passed':canary_ok,'oracle':c['oracle_status'],'required_alarm':alarm}
            result['status']='VERIFIED_REPAIR' if good==3 and canary_ok else 'FAILED_REPAIR'
    (out/'grade.json').write_text(json.dumps(result,indent=2)+'\n')
    return result

if __name__=='__main__':
    p=argparse.ArgumentParser()
    p.add_argument('--trial',required=True)
    p.add_argument('--xezim',required=True)
    p.add_argument('--fault-source',required=True)
    p.add_argument('--out',required=True)
    p.add_argument('--seed',type=int,default=7103)
    a=p.parse_args()
    r=grade(a.trial,a.xezim,a.fault_source,a.out,a.seed)
    print(json.dumps(r,indent=2))
    sys.exit(0 if r['status'] in ('VERIFIED_REPAIR','VERIFIED_CLEAN') else 1)
