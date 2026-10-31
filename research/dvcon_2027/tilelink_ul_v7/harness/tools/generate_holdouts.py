#!/usr/bin/env python3
"""Create a private, post-freeze 12-fault/2-clean instance catalog exactly once.

This is generation infrastructure, not a supplied novel-fault taxonomy. It does
not invent a review, admit faults or start any experimental model process.
"""
from __future__ import annotations
import argparse,hashlib,json,os,random,re,secrets,sys,datetime
from pathlib import Path
from freeze import ROOT,verify as verify_freeze
from primary_contract import SCHEMA,inside,load,digest,validate_taxonomy,checked_file
from generate_stimulus import write as write_stimulus
from generate_prospective_stimulus import write as write_temporal_stimulus

def substitute(text,params):
    for key,value in params.items():text=text.replace('{{'+key+'}}',str(value))
    if '{{' in text:raise ValueError('unresolved template parameter')
    return text

def ref(root,p):return {'path':str(p.relative_to(root)),'sha256':digest(p)}
def save(p,value):
    with p.open('x') as f:
        json.dump(value,f,indent=2);f.write('\n');f.flush();os.fsync(f.fileno())
    p.chmod(0o400)

def generate(root,out,freeze,seed):
    """Trusted orchestration only. CLI verifies the freeze before entering here."""
    root=Path(root).resolve();out=Path(out).resolve()
    if not out.is_relative_to(root/'private') or out==root/'private':raise ValueError('generation must stay in a new private subdirectory')
    if len(seed)!=32:raise ValueError('generation requires a private 256-bit seed')
    prereg=load(root/'config/preregistration.json');tax=load(checked_file(root,prereg['taxonomy']));validate_taxonomy(root,tax)
    # Freeze identity is checked again by the caller before creation; templates
    # remain private and only a case-specific source is later disclosed.
    out.mkdir(parents=True,exist_ok=False);out.chmod(0o700)
    rng=random.Random(int.from_bytes(seed,'big'));specs=list(tax['cases']);rng.shuffle(specs)
    entries=[]
    try:
        for index,spec in enumerate(specs):
            case_id='case_'+hashlib.sha256(seed+b':'+str(index).encode()).hexdigest()[:16]
            dest=out/case_id;dest.mkdir(mode=0o700)
            parameters={key:rng.randint(bounds['min'],bounds['max']) for key,bounds in sorted(spec.get('parameters',{}).items())}
            clean=substitute(checked_file(root,spec['clean_template']).read_text(),parameters)
            source=clean
            edit=None
            if spec['kind']=='fault':
                old=substitute(spec['edit']['old'],parameters);new=substitute(spec['edit']['new'],parameters)
                if clean.count(old)!=1:raise ValueError('instantiated fault anchor not unique')
                source=clean.replace(old,new,1);edit={'old':new,'new':old}
            clean_path=dest/'clean.sv';source_path=dest/'case.sv'
            for path,text in [(clean_path,clean),(source_path,source)]:
                with path.open('x') as stream:stream.write(text)
                path.chmod(0o400)
            configs={}
            for label,path in [('case',source_path),('clean',clean_path)]:
                config={'role':spec['component'],'source':str(path),'tb':str(checked_file(root,spec['testbench'])),'extra_sources':[str(checked_file(root,r)) for r in spec.get('extra_sources',[])],'patch_target':spec['patch_target'],'eligibility':'PRIMARY_GENERATED_NOT_ADMITTED'}
                config_path=dest/(label+'_config.json');save(config_path,config);configs[label]=ref(root,config_path)
            schedules={}
            for label in ('original','unseen','adversarial'):
                stim=dest/('stim_'+label)
                writer=write_temporal_stimulus if tax.get('stimulus_profile')=='temporal_reuse_v2' else write_stimulus
                meta=writer(rng.randrange(1,2**31),tax.get('transactions_per_schedule',192),stim)
                meta['check_monitor_claims']=True
                (stim/'stimulus.json').write_text(json.dumps(meta,indent=2)+'\n')
                for path in stim.iterdir():path.chmod(0o400)
                schedules[label]={'hex':ref(root,stim/'stimulus.hex'),'metadata':ref(root,stim/'stimulus.json')}
            entries.append({'case_id':case_id,'private_spec_id':spec['spec_id'],'kind':spec['kind'],'component':spec['component'],'mechanism_family':spec.get('mechanism_family'),'mechanism_private':spec['mechanism_private'],'expected_symptom':spec['expected_symptom'],'identifiability':spec['identifiability'],'novelty_review_questions':spec['novelty_review_questions'],'parameters':parameters,'source':ref(root,source_path),'clean_source':ref(root,clean_path),'case_manifest':configs['case'],'clean_case_manifest':configs['clean'],'schedules':schedules,'expected_repair_private':edit})
        catalog={'schema':SCHEMA,'status':'GENERATED_NOT_ADMITTED','created_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'timestamp_scope':'local host clock, not an external attestation','freeze_sha256':digest(root/'config/freeze_receipt.json'),'frozen_inputs':freeze['protected_inputs'],'author_id':tax['author_id'],'cases':entries}
        save(out/'generation_seed_private.json',{'seed_hex':seed.hex(),'scope':'Private reproducibility seed, never a model input or public receipt'})
        save(out/'catalog.json',catalog)
        save(out/'generation_receipt.json',{'status':'GENERATED_NOT_ADMITTED','catalog_sha256':digest(out/'catalog.json'),'freeze_sha256':catalog['freeze_sha256'],'cases':14,'faults':12,'clean_controls':2,'primary_subjects_started':0,'review_complete':False})
        return catalog
    except Exception as exc:
        save(out/'generation_failure.json',{'status':'FAILED_GENERATION','error':type(exc).__name__+': '+str(exc),'action':'Preserve incomplete output; do not use it for primary subjects or silently regenerate under the same frozen cohort'})
        raise

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',required=True);a=p.parse_args()
    frozen=verify_freeze(required=True)
    # A persistent claim prevents outcome-dependent regeneration after any
    # attempt, including a partial generation failure.
    claim=ROOT/'private/primary_generation_claim.json'
    with claim.open('x') as stream:
        json.dump({'out':str(Path(a.out).resolve()),'freeze_sha256':digest(ROOT/'config/freeze_receipt.json'),'status':'GENERATION_STARTED'},stream);stream.write('\n');stream.flush();os.fsync(stream.fileno())
    claim.chmod(0o400)
    generate(ROOT,Path(a.out),frozen,secrets.token_bytes(32))
    print('GENERATED_NOT_ADMITTED: 14 private instances; no model subjects launched')
if __name__=='__main__':main()
