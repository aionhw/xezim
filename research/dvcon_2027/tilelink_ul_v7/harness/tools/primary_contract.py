"""Trusted-host primary preregistration and admission checks.

These checks bind supplied review records to artifacts. They cannot certify a
reviewer's identity or replace independent scientific judgment.
"""
from __future__ import annotations
import hashlib,json,re
from pathlib import Path

SCHEMA='TLUL_PRIMARY_V1'
ROLES={'endpoint','traffic','monitor','scoreboard'}
TARGETS={'rtl/tlul_mem_endpoint.sv','rtl/tlul_components.sv'}
LABELS=('original','unseen','adversarial')
FUNCTIONAL_ISSUES={'a_stall_payload_instability','d_stall_payload_instability','unexpected_extra_a','a_intent_mismatch','source_reused_while_pending','invalid_profile_request','invalid_full_word_mask','empty_partial_mask','orphan_d_response','d_metadata_mismatch','read_data_mismatch','monitor_false_alarm'}
def concrete_symptom(oracle):return any(i.get('kind') in FUNCTIONAL_ISSUES and type(i.get('cycle')) is int for i in oracle.get('issues',[]))

def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def canonical_hash(obj):return hashlib.sha256(json.dumps(obj,sort_keys=True,separators=(',',':')).encode()).hexdigest()
def load(path):return json.loads(Path(path).read_text())
def inside(root,name):
    root=Path(root).resolve();path=Path(name)
    if path.is_absolute() or '..' in path.parts:raise ValueError('expected project-relative artifact path')
    p=root/path
    if p.is_symlink() or not p.resolve().is_relative_to(root):raise ValueError('artifact escapes isolated project')
    return p

def checked_file(root,entry):
    if not isinstance(entry,dict) or set(entry)!={'path','sha256'}:raise ValueError('expected path/sha256 artifact reference')
    path=inside(root,entry['path'])
    if not re.fullmatch('[0-9a-f]{64}',str(entry['sha256'])) or not path.is_file() or digest(path)!=entry['sha256']:
        raise ValueError('artifact content hash mismatch: '+str(entry['path']))
    return path

def validate_taxonomy(root,taxonomy):
    if taxonomy.get('schema')!=SCHEMA:raise ValueError('unknown taxonomy schema')
    if taxonomy.get('status')!='FINAL_DESIGN':raise ValueError('taxonomy is not finalized')
    if not taxonomy.get('author_id') or not taxonomy.get('identifiability_policy'):
        raise ValueError('taxonomy author and identifiability policy required')
    cases=taxonomy.get('cases',[])
    if len(cases)!=14 or sum(c.get('kind')=='fault' for c in cases)!=12 or sum(c.get('kind')=='clean' for c in cases)!=2:
        raise ValueError('taxonomy requires twelve faults and two clean controls')
    ids=[c.get('spec_id') for c in cases]
    if len(set(ids))!=14 or not all(isinstance(c,str) and re.fullmatch(r'[A-Za-z0-9_]+',c) for c in ids):raise ValueError('unique taxonomy identifiers required')
    families=[c.get('mechanism_family') for c in cases if c['kind']=='fault']
    if len(set(families))!=12 or not all(families):raise ValueError('twelve distinct private mechanism families required')
    if len({c.get('component') for c in cases if c['kind']=='fault'})<3:raise ValueError('faults require at least three component categories')
    if taxonomy.get('stimulus_profile','legacy_random_v1') not in ('legacy_random_v1','temporal_reuse_v2'):raise ValueError('unrecognized frozen stimulus profile')
    count=taxonomy.get('transactions_per_schedule',192)
    if type(count) is not int or not 96<=count<=512:raise ValueError('schedule count must be in96..512')
    refs=[]
    for case in cases:
        if case.get('component') not in ROLES or case.get('patch_target') not in TARGETS:raise ValueError('unknown component or patch target')
        for field in ('mechanism_private','expected_symptom','identifiability','novelty_review_questions'):
            if not case.get(field):raise ValueError('missing private case declaration: '+field)
        refs.extend([case['clean_template'],case['testbench'],*case.get('extra_sources',[])])
        if case['kind']=='fault':
            edit=case.get('edit',{})
            if not isinstance(edit.get('old'),str) or len(edit['old'])<5 or not isinstance(edit.get('new'),str) or edit['new']==edit['old']:
                raise ValueError('fault requires a concrete unique edit')
        elif case.get('edit') is not None:raise ValueError('clean controls must not have an edit')
        params=case.get('parameters',{})
        if not isinstance(params,dict):raise ValueError('parameters must be explicit integer ranges')
        for name,bounds in params.items():
            if not re.fullmatch('[A-Z][A-Z0-9_]*',name) or not isinstance(bounds,dict) or set(bounds)!={'min','max'} or any(type(bounds[k]) is not int for k in bounds) or not 0<=bounds['min']<=bounds['max']<=65535:
                raise ValueError('invalid private parameter range')
        text=checked_file(root,case['clean_template']).read_text()
        spans=[text]+([case['edit']['old'],case['edit']['new']] if case['kind']=='fault' else [])
        tokens=set(re.findall(r'\{\{([A-Z][A-Z0-9_]*)\}\}', '\n'.join(spans)))
        if tokens!=set(params):raise ValueError('template parameters do not match declared ranges')
        if case['kind']=='fault' and text.count(case['edit']['old'])!=1:raise ValueError('fault edit anchor is not unique in clean template')
        # Avoid silently leaving malformed substitution delimiters in executable source.
        for span in spans:
            if '{{' in re.sub(r'\{\{[A-Z][A-Z0-9_]*\}\}','',span):raise ValueError('invalid parameter token')
    return {ref['path']:digest(checked_file(root,ref)) for ref in refs}

def validate_preregistration(root,protocol):
    if protocol.get('schema')!=SCHEMA or protocol.get('status')!='FINAL_PROTOCOL':raise ValueError('finalized preregistration required')
    refs={name:checked_file(root,protocol.get(name)) for name in ('rubric','taxonomy','holdout_generator')}
    if refs['rubric']!=inside(root,'protocol/STAGE1_RUBRIC.md') or refs['holdout_generator']!=inside(root,'tools/generate_holdouts.py'):
        raise ValueError('rubric/generator must bind the actual execution files')
    inputs=validate_taxonomy(root,load(refs['taxonomy']))
    expected={**inputs,**{protocol[k]['path']:protocol[k]['sha256'] for k in refs}}
    if protocol.get('protected_inputs')!=expected:raise ValueError('protected input inventory is incomplete or inconsistent')
    if protocol.get('analysis_seed')!=71032026 or protocol.get('bootstrap_resamples')!=10000:raise ValueError('analysis settings differ from implemented preregistered analysis')
    for field in ('identifiability_policy','independent_review_plan','author_id','prior_family_exclusion_policy'):
        if not protocol.get(field):raise ValueError('missing preregistration field: '+field)
    if protocol['author_id']!=load(refs['taxonomy'])['author_id']:raise ValueError('author identity inconsistent')
    if protocol.get('allocation')!={'faults':12,'clean_controls':2,'modes':['A','B','C','D'],'repeats':3,'sessions':168}:raise ValueError('primary allocation differs from PLAN')
    if protocol.get('limits')!={'tool_calls':12,'evidence_bytes':131072,'trial_seconds':1200}:raise ValueError('primary limits differ from qualified briefing')
    config=load(inside(root,'config/model.json'))
    if (protocol.get('model'),protocol.get('reasoning_effort'))!=(config['model'],config['reasoning_effort']):raise ValueError('preregistered model differs from runner')
    return expected

def verify_generated(root,catalog_path,freeze):
    catalog_path=Path(catalog_path).resolve()
    if not catalog_path.is_relative_to(Path(root).resolve()/'private'):raise ValueError('primary catalog must remain private within this project')
    catalog=load(catalog_path)
    if catalog.get('schema')!=SCHEMA or catalog.get('freeze_sha256')!=digest(inside(root,'config/freeze_receipt.json')):
        raise ValueError('catalog does not match current pre-generation freeze')
    if catalog.get('frozen_inputs')!=freeze.get('protected_inputs'):raise ValueError('catalog design identity mismatch')
    entries=catalog.get('cases',[])
    if len(entries)!=14 or len({c.get('case_id') for c in entries})!=14 or sum(c.get('kind')=='fault' for c in entries)!=12 or sum(c.get('kind')=='clean' for c in entries)!=2:raise ValueError('invalid generated allocation')
    for c in entries:
        for field in ('source','clean_source','case_manifest','clean_case_manifest'):
            checked_file(root,c[field])
        if set(c['schedules'])!=set(LABELS):raise ValueError('three generated schedules required')
        for label,stim in c['schedules'].items():
            hexfile=checked_file(root,stim['hex']);meta=load(checked_file(root,stim['metadata']))
            if meta.get('stimulus_sha256')!=digest(hexfile):raise ValueError('stimulus metadata hash mismatch')
    return catalog

def verify_admission(root,catalog_path,admission_path,freeze):
    catalog=verify_generated(root,catalog_path,freeze);admission=load(admission_path)
    if admission.get('schema')!=SCHEMA or admission.get('status')!='APPROVED' or admission.get('catalog_sha256')!=digest(catalog_path):raise ValueError('independent admission not approved for this catalog')
    review=admission.get('review',{})
    prereg=load(inside(root,'config/preregistration.json'))
    if review.get('origin')!='human' or review.get('independent_of_construction') is not True or not review.get('reviewer_id') or review['reviewer_id']==prereg['author_id'] or not review.get('utc') or not review.get('expertise'):
        raise ValueError('independent human admission review required')
    decisions=admission.get('cases',{})
    if set(decisions)!={c['case_id'] for c in catalog['cases']}:raise ValueError('all fourteen admission decisions required')
    for c in catalog['cases']:
        decision=decisions[c['case_id']]
        if decision.get('admitted') is not True or decision.get('novelty_review_passed') is not True or decision.get('identifiability_class')!=c['identifiability'] or not decision.get('reason'):
            raise ValueError('case admission/novelty/identifiability incomplete')
        # Hash-bound automated evidence is generated by the separate admission runner.
        evidence=load(checked_file(root,decision.get('execution_evidence')))
        if evidence.get('status')!='PASS' or evidence.get('catalog_sha256')!=digest(catalog_path) or evidence.get('case_id')!=c['case_id']:
            raise ValueError('case execution admission evidence failed or mismatched')
        expected={(variant,label,sim) for variant in ('clean','case') for label in LABELS for sim in ('stock','research','reference')}
        records=evidence.get('runs',[])
        public_traces={}
        if len(records)!=18 or {(r.get('variant'),r.get('schedule'),r.get('simulator')) for r in records}!=expected:raise ValueError('18 clean/case x3 schedule x3 simulator admissions required')
        for record in records:
            receipt=load(checked_file(root,record['receipt']));oracle=load(checked_file(root,record['oracle']));log=checked_file(root,record['stdout'])
            required='PASS' if record['variant']=='clean' or c['kind']=='clean' else 'FAIL'
            from monitor_oracle import evaluate
            metadata=load(inside(root,c['schedules'][record['schedule']]['metadata']['path']))
            if evaluate(log.read_text(),metadata)!=oracle:raise ValueError('saved admission oracle differs from independent recomputation')
            public_traces[(record['variant'],record['schedule'],record['simulator'])]=[s for s in log.read_text().splitlines() if s.startswith(('PIN|','MON|','SIM_END|'))]
            source=c['clean_source'] if record['variant']=='clean' else c['source']
            if receipt.get('exit_code')!=0 or receipt.get('timed_out') is not False or receipt.get('source_sha256')!=source['sha256'] or receipt.get('stimulus_sha256')!=c['schedules'][record['schedule']]['hex']['sha256']:
                raise ValueError('admission run did not execute the bound source and stimulus')
            expected_binary=freeze['artifacts']['.deps/'+({'stock':'xezim/target/release/xezim','research':'xezim-observer/target/release/xezim','reference':'iverilog-installed/bin/vvp'}[record['simulator']])]
            if receipt.get('binary_sha256')!=expected_binary:raise ValueError('admission simulator identity differs from freeze')
            if oracle.get('status')!=required or receipt.get('oracle_status')!=required:raise ValueError('clean/fault oracle admission mismatch')
            if required=='FAIL' and not concrete_symptom(oracle):
                raise ValueError('fault needs a concrete functional or monitor symptom')
            if 'SIM_END|' not in log.read_text() or (required=='PASS' and '|drained=1|' not in log.read_text()):raise ValueError('admission must complete; clean runs must drain')
        for variant in ('clean','case'):
            for label in LABELS:
                if not public_traces[variant,label,'stock']==public_traces[variant,label,'research']==public_traces[variant,label,'reference']:raise ValueError('admission simulator traces disagree')
    return catalog,admission

def authorize_subject(root,args,freeze):
    if not getattr(args,'primary_catalog',None) or not getattr(args,'primary_admission',None) or not getattr(args,'case_id',None) or getattr(args,'repeat',None) not in (1,2,3):
        raise ValueError('primary requires reviewed catalog, admission, case ID and repeat')
    if args.timeout!=1200 or args.engineering_external:raise ValueError('primary deadline/tier differs from preregistration')
    catalog,admission=verify_admission(root,args.primary_catalog,args.primary_admission,freeze)
    matches=[c for c in catalog['cases'] if c['case_id']==args.case_id]
    if len(matches)!=1:raise ValueError('case absent from reviewed catalog')
    case=matches[0]
    if Path(args.fault_source).resolve()!=inside(root,case['source']['path']) or not args.case_manifest or Path(args.case_manifest).resolve()!=inside(root,case['case_manifest']['path']) or Path(args.stimuli).resolve()!=inside(root,case['schedules']['original']['hex']['path']).parent:
        raise ValueError('primary arguments do not identify the reviewed case')
    if digest(args.xezim)!=freeze['artifacts']['.deps/xezim-observer/target/release/xezim']:
        raise ValueError('all primary modes require the same qualified research simulator')
    if not Path(args.out).resolve().is_relative_to(Path(root).resolve()/'results'):raise ValueError('primary outputs must stay in isolated results directory')
    expected_id=f"{case['case_id']}_{args.mode}_{args.repeat}"
    if args.trial_id!=expected_id:raise ValueError('primary trial identifier differs from scheduled cell')
    return {'case_id':case['case_id'],'repeat':args.repeat,'catalog_path':str(Path(args.primary_catalog).resolve()),'catalog_sha256':digest(args.primary_catalog),'admission_path':str(Path(args.primary_admission).resolve()),'admission_sha256':digest(args.primary_admission),'freeze_sha256':digest(inside(root,'config/freeze_receipt.json'))}


def reserve_subject(root,authorization,trial_id):
    """Persistent per-cell claim; rerunning a cell cannot replace its outcome."""
    import os
    if not re.fullmatch(r'case_[0-9a-f]{16}_[ABCD]_[123]',trial_id):raise ValueError('invalid primary cell identifier')
    claims=Path(authorization['catalog_path']).parent/'primary_subject_claims';claims.mkdir(exist_ok=True)
    with (claims/(trial_id+'.json')).open('x') as stream:
        json.dump({'trial_id':trial_id,**authorization,'host_process_id':os.getpid()},stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
    (claims/(trial_id+'.json')).chmod(0o400)


def pilot_phase_closed(root):
    root=Path(root)
    return (root/'config/freeze_receipt.json').exists() or (root/'private/primary_generation_claim.json').exists()
