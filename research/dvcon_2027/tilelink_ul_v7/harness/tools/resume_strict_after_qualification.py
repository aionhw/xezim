"""Continue from immutable successful live qualification; no retries of pilot outcomes."""
import datetime,hashlib,json,shutil,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];out=ROOT/'results/strict_continuation_003';out.mkdir(exist_ok=False)
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def state(phase,**extra):
 d={'phase':phase,'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),**extra};(out/'state.json').write_text(json.dumps(d,indent=2)+'\n');print(json.dumps(d),flush=True)
def command(name,args):
 with (out/(name+'.log')).open('x') as f:r=subprocess.run(args,cwd=ROOT,stdout=f,stderr=f)
 if r.returncode:raise RuntimeError(f'{name} failed with exit {r.returncode}; preserved log')
try:
 runtime=ROOT/'.deps/codex_cli_strict/runtime';binary=runtime/'bin/codex';qdir=ROOT/'results/sandbox_validation_strict_001';live=ROOT/'results/tool_allowlist_live_strict_003';q=json.loads((live/'receipt.json').read_text())
 assert json.loads((ROOT/'results/codex_strict_build_007_exit.json').read_text())['exit_code']==0
 assert json.loads((qdir/'receipt.json').read_text())['status']=='PASS'
 assert q['status']=='PASS' and q['codex_sha256']==sha(binary) and q['calls']==['read_evidence'] and not q['violations']
 (ROOT/'results/case_tool_allowlist_qualification.json').write_text(json.dumps(q,indent=2)+'\n');previous=ROOT/'results/case_permissions_qualification.json';archive=ROOT/'results/case_permissions_qualification_stock0166.json'
 if previous.exists() and not archive.exists():shutil.copyfile(previous,archive)
 version=subprocess.check_output([str(binary),'--version'],text=True).strip();permission={'status':'PASS','codex_version':version,'scope':'case + platform/runtime read boundary and compiled case-tool policy; not formal OS security proof','filesystem_network_probe':str((qdir/'receipt.json').relative_to(ROOT)),'actual_thread_probe':str((live/'receipt.json').relative_to(ROOT)),'profile':'dvcon_case','runtime_read_root':str(runtime),'runtime_files':{name:sha(runtime/name) for name in ('bin/codex','codex-resources/bwrap')}};previous.write_text(json.dumps(permission,indent=2)+'\n')
 bp=ROOT/'paper/receipts/strict_runner_build.json';bd=json.loads(bp.read_text());bd.update({'status':'BUILT_AND_LIVE_QUALIFIED','codex_version':version,'runtime_files':permission['runtime_files'],'qualification_receipt':str((live/'receipt.json').relative_to(ROOT)),'tool_presentation':'features.code_mode config with enabled=false and direct_only_tool_namespaces=[functions]','qualified_tool_surface':['read_evidence'],'final_cli_rustc_override':'-C opt-level=0'});bd['build_attempts']['007']='PASS: Cargo exit 0; exact runtime binary filesystem and live case-tool qualification passed';bp.write_text(json.dumps(bd,indent=2)+'\n')
 state('STRICT_MAPPER_GENERATION')
 mapper=ROOT/'results/mapper_strict_001';evaluation=ROOT/'results/mapper_strict_evaluation_001'
 command('mapper',[sys.executable,str(ROOT/'tools/p2_generate_mapper.py'),'--out',str(mapper)])
 command('mapper_evaluation',[sys.executable,str(ROOT/'tools/evaluate_declarative_mapper.py'),'--receipt',str(mapper/'receipt.json'),'--out',str(evaluation)])
 np=ROOT/'results/native_qualification.json';native=json.loads(np.read_text());snapshot=ROOT/'results/native_qualification_before_strict_mapper.json'
 if not snapshot.exists():shutil.copyfile(np,snapshot)
 native['receipts'].append(str((evaluation/'receipt.json').relative_to(ROOT)));native['strict_mapper_receipt']=str((mapper/'receipt.json').relative_to(ROOT));native['mapper_limit']='Model-selected declarative rules; three real schedules plus four bounded missing/ambiguous/same-cycle controls. Not universal unknown/four-state history qualification.';np.write_text(json.dumps(native,indent=2)+'\n')
 state('RUNNING_UNIFORM_STRICT_PILOTS',scheduled_subjects=52)
 command('pilot_schedule',[sys.executable,str(ROOT/'tools/run_strict_pilot_schedule.py')])
 state('STRICT_PILOT_SCHEDULE_FINISHED',scheduled_subjects=52,primary_campaign='NOT_RUN; freeze, independently admitted post-freeze holdouts and expert review remain')
except Exception as e:state('STOPPED_AT_FAILED_GATE',error=type(e).__name__+': '+str(e));raise
