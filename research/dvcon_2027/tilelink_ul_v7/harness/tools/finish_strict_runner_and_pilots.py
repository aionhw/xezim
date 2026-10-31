"""Sequential local continuation after the supervised build; stops on failed gates."""
import datetime,hashlib,json,os,shutil,subprocess,sys,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];out=ROOT/'results'/(sys.argv[1] if len(sys.argv)>1 else 'strict_continuation_001');out.mkdir(exist_ok=False)
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def state(phase,**extra):
 d={'phase':phase,'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),**extra};(out/'state.json').write_text(json.dumps(d,indent=2)+'\n');print(json.dumps(d),flush=True)
def command(name,args):
 with (out/(name+'.log')).open('x') as f:r=subprocess.run(args,cwd=ROOT,stdout=f,stderr=f)
 if r.returncode:raise RuntimeError(f'{name} failed with exit {r.returncode}; preserved log')
try:
 state('WAITING_FOR_BUILD')
 build_exit=ROOT/'results'/(sys.argv[2] if len(sys.argv)>2 else 'codex_strict_build_006_exit.json')
 while not build_exit.is_file():time.sleep(10)
 build=json.loads(build_exit.read_text())
 if build['exit_code']!=0:raise RuntimeError('strict build failed; no model subjects authorized')
 runtime=ROOT/'.deps/codex_cli_strict/runtime';binary=runtime/'bin/codex';state('INSTALLING_LOCAL_RUNTIME')
 shutil.copy2(ROOT/'.deps/codex_strict_target/release/codex',binary)
 command('strip_runtime',['strip','--strip-debug',str(binary)])
 version=subprocess.check_output([str(binary),'--version'],text=True).strip()
 command('login_status',[str(binary),'login','status'])
 if 'ChatGPT' not in (out/'login_status.log').read_text():raise RuntimeError('ChatGPT login status not confirmed')
 command('pro_check',[sys.executable,str(ROOT/'tools/check_pro_auth.py')])
 state('QUALIFYING_FILESYSTEM_AND_NETWORK',codex_version=version)
 qdir=ROOT/'results/sandbox_validation_strict_001'
 command('sandbox_controls',[sys.executable,str(ROOT/'tools/validate_case_sandbox.py'),'--codex-bin',str(binary),'--runtime-root',str(runtime),'--out',str(qdir)])
 if json.loads((qdir/'receipt.json').read_text())['status']!='PASS':raise RuntimeError('strict runtime filesystem/network controls failed')
 state('QUALIFYING_LIVE_CASE_TOOL_SURFACE')
 live=ROOT/'results/tool_allowlist_live_strict_001'
 command('live_tool_qualification',[sys.executable,str(ROOT/'tools/qualify_case_tool_allowlist.py'),'--codex-bin',str(binary),'--out',str(live)])
 q=json.loads((live/'receipt.json').read_text())
 if q.get('status')!='PASS' or q.get('codex_sha256')!=sha(binary):raise RuntimeError('live tool surface qualification failed')
 (ROOT/'results/case_tool_allowlist_qualification.json').write_text(json.dumps(q,indent=2)+'\n')
 previous=ROOT/'results/case_permissions_qualification.json';archive=ROOT/'results/case_permissions_qualification_stock0166.json'
 if previous.exists() and not archive.exists():shutil.copyfile(previous,archive)
 permission={'status':'PASS','codex_version':version,'scope':'case + platform/runtime read boundary and compiled case-tool policy; not formal OS security proof','filesystem_network_probe':str((qdir/'receipt.json').relative_to(ROOT)),'actual_thread_probe':str((live/'receipt.json').relative_to(ROOT)),'profile':'dvcon_case','runtime_read_root':str(runtime),'runtime_files':{name:sha(runtime/name) for name in ('bin/codex','codex-resources/bwrap')}}
 previous.write_text(json.dumps(permission,indent=2)+'\n')
 build_receipt=ROOT/'paper/receipts/strict_runner_build.json';bd=json.loads(build_receipt.read_text());bd.update({'status':'BUILT_AND_LIVE_QUALIFIED','codex_version':version,'runtime_files':permission['runtime_files'],'build_exit_code':0,'interrupted_build':'results/codex_strict_build_005_interruption.json','qualification_allowed_tools':q['surface']['allowed_tools']});build_receipt.write_text(json.dumps(bd,indent=2)+'\n')
 state('STRICT_MAPPER_GENERATION')
 mapper=ROOT/'results/mapper_strict_001';evaluation=ROOT/'results/mapper_strict_evaluation_001'
 command('mapper',[sys.executable,str(ROOT/'tools/p2_generate_mapper.py'),'--out',str(mapper)])
 command('mapper_evaluation',[sys.executable,str(ROOT/'tools/evaluate_declarative_mapper.py'),'--receipt',str(mapper/'receipt.json'),'--out',str(evaluation)])
 native_path=ROOT/'results/native_qualification.json';native=json.loads(native_path.read_text());snapshot=ROOT/'results/native_qualification_before_strict_mapper.json'
 if not snapshot.exists():shutil.copyfile(native_path,snapshot)
 native['receipts'].append(str((evaluation/'receipt.json').relative_to(ROOT)));native['strict_mapper_receipt']=str((mapper/'receipt.json').relative_to(ROOT));native['mapper_limit']='Model-selected declarative rules; three real schedules plus four bounded missing/ambiguous/same-cycle controls. Not universal unknown/four-state history qualification.';native_path.write_text(json.dumps(native,indent=2)+'\n')
 state('RUNNING_UNIFORM_STRICT_PILOTS',scheduled_subjects=52)
 command('pilot_schedule',[sys.executable,str(ROOT/'tools/run_strict_pilot_schedule.py')])
 state('STRICT_PILOT_SCHEDULE_FINISHED',scheduled_subjects=52,primary_campaign='NOT_RUN; freeze, independently admitted post-freeze holdouts and expert review remain')
except Exception as e:
 state('STOPPED_AT_FAILED_GATE',error=type(e).__name__+': '+str(e));raise
