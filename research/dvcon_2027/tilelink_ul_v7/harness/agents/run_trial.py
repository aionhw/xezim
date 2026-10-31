#!/usr/bin/env python3
"""Run one real Pro/ChatGPT-signed-in Codex subject via app-server.

No OpenAI SDK, OPENAI_API_KEY, paid API client or enforced zero cache usage.
Every trial starts a NEW Codex process and thread. The model receives only
approved dynamic tools; new source is disclosed AFTER Stage I hash sealing.

This is the *single-trial* runner; the prospective 168-session generator and
expert adjudication are not provided in this starter package. Do not call the
engineering-external tier a native C/D study or claim a completed primary trial.
"""
from __future__ import annotations
import re
import argparse, hashlib, json, os, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'tools'))
sys.path.insert(0, str(ROOT/'agents'))
from control_suite import execute as execute_control
from pin_wave import read as read_wave
from freeze import verify as verify_freeze
from primary_contract import authorize_subject, reserve_subject, pilot_phase_closed
from codex_app_server import AppServerError, CodexAppServer, cached_token_observations

CONFIG = json.loads((ROOT/'config/model.json').read_text(encoding='utf-8'))
MODEL = CONFIG['model']
EFFORT = CONFIG['reasoning_effort']
MAX_TOOLS = CONFIG['max_tool_actions_per_trial']
MAX_BYTES = CONFIG['max_returned_tool_evidence_bytes_per_trial']


def sha(text: str) -> str:
    return hashlib.sha256(text.encode('utf-8')).hexdigest()


def dump(obj) -> str:
    return json.dumps(obj, sort_keys=True, separators=(',', ':'), default=str)


def parse_json_answer(text: str) -> dict:
    s = text.strip()
    if s.startswith('```'):
        parts = s.splitlines()
        s = '\n'.join(parts[1:-1]).strip()
    value = json.loads(s)
    if not isinstance(value, dict):
        raise ValueError('model output must be a JSON object')
    return value


def tool_def(name, description, properties, required):
    return {'name':name,'description':description,
            'inputSchema':{'type':'object','properties':properties,
                'required':required,'additionalProperties':False}}


class Host:
    """Only host-controlled, case-specific evidence; never a private-path API."""
    def __init__(self, args, out):
        self.args=args
        self.out=out
        self.actions=[]
        self.issued_evidence={}
        self.returned_bytes=0
        self.query_types=[]
        self.interventions=0
        self.stage='I'
        self.baseline=Path(args.baseline).resolve()
        self.source=Path(args.fault_source).resolve()
        self.case_config=json.loads(args.case_manifest.read_text()) if getattr(args,'case_manifest',None) else {}

    def schemas(self):
        kinds = ['pin','wave']
        if self.args.mode in 'BCD':kinds.append('value')
        if self.args.mode in 'CD':kinds.append('semantic')
        tools=[tool_def('read_evidence',
            'Read case-local actual simulator evidence. start must be a nonnegative integer; '
            'count must be an integer from 1 through 40. Each request counts toward the '
            '12-call total. Return citation_id; cite it in Stage I.',
            {'kind':{'type':'string','enum':kinds},
             'start':{'type':'integer','minimum':0},
             'count':{'type':'integer','minimum':1,'maximum':40}},
            ['kind','start','count'])]
        if self.args.mode=='D':
            tools.append(tool_def('run_controlled_experiment',
                'Run a real agent-selected timed force/release on an approved continuous input, '
                'with TWO separate negative controls. at_cycle is a negedge counter integer '
                'from 4 through 2000; duration_cycles is an integer from 1 through 32. '
                'Each request counts toward the 12-call total. Supply prediction BEFORE experiment.',
                {'target':{'type':'string','enum':['request_throttle','response_backpressure']},
                 'at_cycle':{'type':'integer','minimum':4,'maximum':2000},
                 'duration_cycles':{'type':'integer','minimum':1,'maximum':32},
                 'prediction':{'type':'string','minLength':12}},
                ['target','at_cycle','duration_cycles','prediction']))
        return tools

    def _read(self, kind, start, count):
        if not isinstance(start,int) or isinstance(start,bool) or start<0:
            return {'error':'invalid start index'}
        if not isinstance(count,int) or isinstance(count,bool) or count<1 or count>40:
            return {'error':'count must be between 1 and 40'}
        if kind=='pin':
            lines=[s for s in (self.baseline/'stdout.log').read_text().splitlines() if s.startswith(('PIN|','MON|'))]
            return {'kind':'pin','records':lines[start:start+count], 'total_available':len(lines)}
        if kind=='wave':
            path=self.baseline/'wave.vcd'
            if not path.is_file():return {'error':'actual VCD unavailable'}
            rows=read_wave(path)
            return {'kind':'public_pin_vcd','records':rows[start:start+count],'total_available':len(rows),'not_native':True}
        if kind=='value' and self.args.mode in 'BCD':
            lines=[s for s in (self.baseline/'stderr.log').read_text().splitlines()
                   if re.match(r'^\[value-trace\] t=\d+ ', s)]
            if not lines:
                return {'error':'value trace absent, not merely an enabled flag'}
            lines=[re.sub(r'at [^;\n]+?\.sv(?::[0-9]+)?', 'at case.sv', line) for line in lines]
            return {'kind':'value','records':lines[start:start+count], 'total_available':len(lines)}
        if kind=='semantic' and self.args.mode in 'CD':
            if self.args.native_required:
                lines=(self.baseline/'native_events.jsonl').read_text().splitlines()
                records=[json.loads(s) for s in lines]
                for record in records:record['run_id']='case_capture'
                return {'kind':'native_sampled_observation','not_native':False,
                        'records':records[start:start+count], 'raw_total_records':len(records)}
            data=json.loads((self.baseline/'semantic_external.json').read_text())
            return {'kind':'external_adapter_UNSCORED','not_native':True,
                    'records':data['events'][start:start+count],
                    'truncated':data.get('truncated')}
        return {'error':'tool not authorized in this mode'}

    def execute(self, name: str, arguments: dict):
        if not isinstance(arguments,dict):return {'error':'tool arguments must be object'}
        if self.stage!='I':return {'error':'tools closed after Stage I seal'}
        if len(self.actions)>=MAX_TOOLS:return {'error':'tool action limit'}
        kind=None
        if name=='read_evidence':
            if set(arguments)!={'kind','start','count'}:
                value={'error':'incorrect tool argument fields'}
            else:
                value=self._read(arguments['kind'],arguments['start'],arguments['count'])
                if 'error' not in value and value.get('records'):
                    kind=arguments['kind']
        elif name=='run_controlled_experiment' and self.args.mode=='D':
            if set(arguments)!={'target','at_cycle','duration_cycles','prediction'}:
                value={'error':'incorrect intervention argument fields'}
            elif self.interventions>=2:
                value={'error':'intervention count exceeded'}
            else:
                self.interventions+=1
                folder=self.out/f'experiment_{self.interventions:02d}'
                value=execute_control(str(self.args.xezim),self.source,self.args.stimuli,
                        folder,arguments['target'],arguments['at_cycle'],
                        arguments['duration_cycles'],arguments['prediction'],deadline=getattr(self,'deadline',None),case_config=self.case_config)
                if value.get('status')=='PASS' and value.get('actual_runs')==4:
                    kind='experiment'
                else:
                    value={'error':'real control experiments did not qualify','receipt':value}
        else:
            value={'error':'forbidden tool'}
        if 'error' not in value:
            value['citation_id']='tlul_ev_'+str(len(self.issued_evidence)+1).zfill(4)
        packed=dump(value)
        size=len(packed.encode('utf-8'))
        if self.returned_bytes+size>MAX_BYTES:
            value={'error':'model-facing evidence budget exhausted'}
            packed=dump(value)
            size=len(packed.encode('utf-8'))
        else:
            self.returned_bytes+=size
            if 'error' not in value:
                self.issued_evidence[value['citation_id']]=sha(packed)
                if kind:self.query_types.append(kind)
        self.actions.append({'tool':name,'arguments':arguments,'result_sha256':sha(packed),
                             'returned_bytes':size,'success':'error' not in value})
        return value


def validate_baseline(args):
    base=Path(args.baseline)
    if not (base/'stdout.log').is_file() or not (base/'receipt.json').is_file():
        raise ValueError('missing actual simulator stdout.log or receipt.json')
    receipt=json.loads((base/'receipt.json').read_text())
    if args.mode in 'BCD':
        trace = (base/'stderr.log').read_text() if (base/'stderr.log').is_file() else ''
        if not receipt.get('actual_value_trace_marker_found') or not re.search(r'(?m)^\[value-trace\] t=\d+ ', trace):
            raise ValueError('B/C/D require actual value trace records, not a missing/unused tool')
    if args.mode in 'CD':
        args.native_required=not args.engineering_external
        if args.native_required:
            qualification=ROOT/'results/native_qualification.json'
            if not qualification.is_file():
                raise ValueError('C/D native qualification receipt missing')
            q=json.loads(qualification.read_text())
            if q.get('status')!='PASS' or q.get('binary_sha256')!=receipt.get('binary_sha256'):
                raise ValueError('C/D native qualification does not match baseline binary')
            if receipt.get('mode')!='native' or not receipt.get('native_observer_executed'):
                raise ValueError('C/D native requires qualified research simulator receipt')
            for name in ('native_events.jsonl','native_stats.json','native_manifest.json'):
                if not (base/name).is_file():raise ValueError('missing native artifact '+name)
        elif not (base/'semantic_external.json').is_file():
            raise ValueError('engineering-only external mapping requires actual adapter file')
    else:
        args.native_required=False
    if not Path(args.fault_source).is_file():raise ValueError('fault source file missing')
    if not Path(args.stimuli,'stimulus.json').is_file():raise ValueError('stimulus missing')
    file_sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
    for key,path in [('source_sha256',args.fault_source),('binary_sha256',args.xezim),('stimulus_sha256',Path(args.stimuli)/'stimulus.hex')]:
        if receipt.get(key)!=file_sha(path):
            raise ValueError('case bundle identity mismatch: '+key)
    config=json.loads(Path(args.case_manifest).read_text()) if getattr(args,'case_manifest',None) else {}
    if receipt.get('case_config',{})!=config:
        raise ValueError('case bundle configuration mismatch')
    tb=Path(config.get('tb',ROOT/'rtl/tb_tlul.sv'))
    if receipt.get('tb_sha256')!=file_sha(tb):
        raise ValueError('case bundle testbench identity mismatch')
    expected_extra={str(Path(p).resolve()):file_sha(p) for p in config.get('extra_sources',[])}
    if expected_extra and receipt.get('extra_source_sha256')!=expected_extra:
        raise ValueError('case bundle dependency identity mismatch')
    if not (base/'wave.vcd').is_file() or not read_wave(base/'wave.vcd'):
        raise ValueError('case bundle requires qualified public-pin VCD')
    if args.engineering_external and args.mode not in 'CD':
        raise ValueError('--engineering-external is valid only for modes C/D')
    if args.mode=='D' and not Path(args.xezim).is_file():raise ValueError('simulator binary missing')
    return receipt


def stage1_instructions(args, symptom):
    spec=(ROOT/'protocol/TLUL_PROFILE.md').read_text()
    if getattr(args,'case_manifest',None):
        spec+='\nThis case has public MON claim records sampled with PIN records. The source-balance monitor must track accepted A sources until their accepted D retirement; the shadow scoreboard must compare accepted read responses to byte-memory snapshots at A acceptance. Both must be silent on legal correct PIN traffic. Traffic delivery adapters must preserve accepted requests.\n'
    contract=(ROOT/'protocol/OBSERVATION_CONTRACT.md').read_text()
    return (
        'You are a fresh, blinded TileLink-UL hardware debugging subject. Do not assume '
        'a bug exists. The implementation source is NOT available in Stage I. '
        'Use only the case-specific host tools for evidence; do not invoke shell, file, '
        'web, MCP or delegation tools. Source and older benchmark transcripts are forbidden. '
        f'Your total trial budget is {MAX_TOOLS} tool calls, {MAX_BYTES} returned evidence '
        f'bytes, and {getattr(args,"timeout",1200)} seconds across both stages. Invalid '
        'requests count toward the call limit. Read start must be a nonnegative integer '
        'and read count must be an integer from 1 through 40. Intervention at_cycle '
        'must be an integer from 4 through 2000 and duration_cycles from 1 through 32. '
        'Budget your queries and finish Stage I within these limits. If evidence remains '
        'insufficient, report that uncertainty with valid issued citations; do not exceed '
        'the call limit. Required Stage I use: A must query pin; B must query value; '
        'C must query native semantic; D must query native semantic and execute a '
        'qualifying experiment with both controls. '
        'The evidence tools return citation_id values. Do not invent citations or provenance. '
        'Mode D: select a legal timing experiment, state a discriminating prediction '
        'before the tool call, and examine its two negative controls. '\
        'Do NOT propose source patches until Stage II. Return a SINGLE JSON object with '
        'assessment, component, mechanism, two_alternative_hypotheses, '
        'supporting_issued_evidence (list of citation IDs), predicted_discriminating_result, '
        'and confidence. No Markdown.\n\n'
        f'Protocol specification:\n{spec}\n\nObserver contract:\n{contract}\n\n'
        f'Common symptom (identical across modes):\n{dump(symptom)}\n'
        f'Your authorized mode: {args.mode}. First query actual evidence.')


def run_subject(args, *, codex='codex'):
    if getattr(args,'cohort','pilot')!='primary' and pilot_phase_closed(ROOT):
        raise ValueError('pilot inference is closed after prospective freeze or generation; do not expose holdouts under a pilot label')
    frozen=verify_freeze(required=getattr(args,'cohort','pilot')=='primary')
    primary=authorize_subject(ROOT,args,frozen) if getattr(args,'cohort','pilot')=='primary' else None
    validate_baseline(args)
    if primary:reserve_subject(ROOT,primary,args.trial_id)
    out=Path(args.out).resolve()
    if out.exists():raise FileExistsError('create-once trial directory; do not overwrite '+str(out))
    out.mkdir(parents=True)
    raw=out/'raw';raw.mkdir()
    isolated_cwd=out/'subject_workspace';isolated_cwd.mkdir()
    # Source is not copied into model's cwd in either stage.
    record={'trial_id':args.trial_id,'cohort':getattr(args,'cohort','pilot'),'mode':args.mode,'auth_requested':'ChatGPT Pro via Codex login',
        'model_requested':MODEL,'effort_requested':EFFORT,
        'cache_policy':'provider_cache_reads_permitted_and_reported',
        'fresh_process':True,'fresh_thread':True,'resume_or_fork':False,
        'semantic_tier':'native_research' if args.native_required else
            ('external_UNSCORED' if args.mode in 'CD' else 'conventional'),
        'stimuli_path':str(Path(args.stimuli).resolve()),'status':'IN_PROGRESS','stages':{},'cache_observations':[]}
    host=Host(args,out)
    if primary:record['primary_authorization']=primary
    record['case_config']=host.case_config
    record['patch_target']=host.case_config.get('patch_target','rtl/tlul_mem_endpoint.sv')
    report=json.loads((Path(args.baseline)/'oracle.json').read_text())
    issue=(report.get('issues') or [{}])[0]
    symptom={'status':report.get('status'),
        'first_issue_kind':issue.get('kind'),'first_issue_cycle':issue.get('cycle')}
    record['runtime_code_sha256']={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for folder in ('agents','tools','protocol','config') for p in (ROOT/folder).rglob('*') if p.is_file() and '__pycache__' not in str(p)}
    began=time.monotonic()
    host.deadline=began+args.timeout
    try:
        with CodexAppServer(codex=codex,stderr_path=raw/'codex_stderr.log',
                            event_path=raw/'codex_events.jsonl', workspace=isolated_cwd) as app:
            app.initialize()
            record['process_id']=app.proc.pid
            record['codex_binary_sha256']=hashlib.sha256(Path(codex).resolve().read_bytes()).hexdigest()
            record['initialize']=app.initialize_result
            record['workspace_initial_inventory']=[]
            record['service_preflight']=app.preflight(MODEL,EFFORT)
            app.handler=host.execute
            thread_id=app.start_thread(isolated_cwd,MODEL,host.schemas())
            record['thread_id']=thread_id
            deadline=began+args.timeout
            first=app.turn(thread_id=thread_id,cwd=isolated_cwd,model=MODEL,
                effort=EFFORT,text=stage1_instructions(args,symptom),timeout=max(1,deadline-time.monotonic()))
            if time.monotonic()>=deadline:raise ValueError('trial deadline exceeded before seal')
            stage1=parse_json_answer(first['text'])
            fields={'assessment','component','mechanism','two_alternative_hypotheses','supporting_issued_evidence','predicted_discriminating_result','confidence'}
            if not fields.issubset(stage1):raise ValueError('Stage I schema incomplete')
            citations=stage1.get('supporting_issued_evidence')
            if not isinstance(citations,list) or not citations or any(
                    not isinstance(c,str) or c not in host.issued_evidence for c in citations):
                raise ValueError('Stage I missing/invalid issued evidence citation(s)')
            if app.violation:
                raise AppServerError('capability violation before source disclosure')
            required={'A':'pin','B':'value','C':'semantic','D':'experiment'}[args.mode]
            if required not in host.query_types:
                raise ValueError('required mode evidence unused before source reveal')
            if args.mode=='D' and 'semantic' not in host.query_types:
                raise ValueError('D native query missing before source reveal')
            first_hash=sha(dump(stage1))
            sealed={'turn_id':first['turn_id'],'stage1':stage1,'sha256':first_hash,
                 'issued_evidence_hashes':dict(host.issued_evidence),
                 'query_types':list(host.query_types)}
            seal_path=out/'stage1_sealed.json'
            with seal_path.open('x') as stream:
                json.dump(sealed,stream,indent=2)
                stream.write('\n')
                stream.flush()
                os.fsync(stream.fileno())
            seal_path.chmod(0o444)
            host.stage='II'
            record['stages']['I']={'turn_id':first['turn_id'],'sha256':first_hash,
                'sealed_before_source':True}
            if time.monotonic()>=deadline:
                raise ValueError('trial deadline exceeded before Stage II disclosure')
            # Only AFTER immutable seal: the current permitted fault-bearing source.
            source_text=Path(args.fault_source).read_text()
            stage2_prompt=(
                'Stage I is sealed by the trusted host. Tools are closed. '
                'You may now read the CURRENT implementation only, and propose ONE '
                'minimal patch to the permitted target below. Tests, the independent '
                'oracle, stimulus and other artifacts are protected. If the permitted '
                'target is a monitor or scoreboard, repair it without weakening '
                'its checks. Reply with JSON: '
                '{"path":'+json.dumps(record['patch_target'])+',"old":"exact span",'
                '"new":"replacement","explanation":"..."}. '
                'If no defect is present, return {"path":null,"old":null,"new":null,'
                '"explanation":"clean control"}. No Markdown.\n\n'
                f'CURRENT PERMITTED SOURCE:\n{source_text}')
            second=app.turn(thread_id=thread_id,cwd=isolated_cwd,model=MODEL,
                effort=EFFORT,text=stage2_prompt,timeout=max(1,deadline-time.monotonic()))
            patch=parse_json_answer(second['text'])
            if Path(codex).name in ('codex','codex.exe'):
                record['tool_surface_audit']=app.verify_tool_surface()
            record['stages']['II']={'turn_id':second['turn_id'],'submission':patch}
            record['cache_observations']=cached_token_observations(app.events)
            record['service_events_recorded']=len(app.events)
            record['server_tool_requests_recorded']=sum(r.get('method')=='item/tool/call' for r in app.server_requests)
            record['actual_query_types']=list(host.query_types)
            record['actions']=host.actions
            mandatory={'A':'pin' in host.query_types,'B':'value' in host.query_types,
                'C':'semantic' in host.query_types,
                'D':'semantic' in host.query_types and 'experiment' in host.query_types}[args.mode]
            record['mandatory_mode_exercised']=mandatory
            test_binary=Path(codex).name not in ('codex','codex.exe')
            record['status']=('UNSCORED_TEST_TRANSPORT' if test_binary else
                ('COMPLETED_UNSCORED_ENGINEERING' if args.engineering_external else
                    ('COMPLETED' if mandatory else 'COMPLETED_WITH_REQUIRED_TOOL_UNUSED')))
    except Exception as exc:
        record['status']='FAILED'
        record['error']=type(exc).__name__+': '+str(exc)
        record['actions']=host.actions
        record['actual_query_types']=list(host.query_types)
    finally:
        if 'app' in locals():
            record['cache_observations']=cached_token_observations(app.events)
            record['cache_telemetry_reported']=bool(record['cache_observations'])
        record['elapsed_seconds']=time.monotonic()-began
        (out/'trial.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


def build_parser():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--mode',choices=['A','B','C','D'],required=True)
    p.add_argument('--baseline',type=Path,required=True)
    p.add_argument('--fault-source',type=Path,required=True)
    p.add_argument('--cohort',choices=['pilot','primary'],default='pilot')
    p.add_argument('--xezim',type=Path,required=True)
    p.add_argument('--stimuli',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--trial-id',required=True)
    p.add_argument('--case-manifest',type=Path)
    p.add_argument('--primary-catalog',type=Path)
    p.add_argument('--primary-admission',type=Path)
    p.add_argument('--case-id')
    p.add_argument('--repeat',type=int,choices=[1,2,3])
    p.add_argument('--codex-bin',default=str(ROOT/CONFIG['codex_cli_path']))
    p.add_argument('--timeout',type=int,default=1200)
    p.add_argument('--engineering-external',action='store_true',
        help='C/D with external adapter ONLY; automatic UNSCORED output')
    return p


def main():
    args=build_parser().parse_args()
    rec=run_subject(args,codex=args.codex_bin)
    print(json.dumps({k:rec.get(k) for k in ('status','trial_id','model_requested',
                            'effort_requested','mandatory_mode_exercised')},indent=2))
    sys.exit(0 if rec['status'] in ('COMPLETED','COMPLETED_UNSCORED_ENGINEERING') else 1)

if __name__=='__main__':main()
