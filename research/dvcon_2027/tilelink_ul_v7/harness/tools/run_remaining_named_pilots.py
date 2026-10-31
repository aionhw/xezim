"""Run each remaining admitted named pilot once per mode; never replace failures."""
import json,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];ledger=ROOT/'results/remaining_named_pilot_schedule_001.jsonl'
with ledger.open('x') as f:
 for case in range(2,6):
  for mode in 'ABCD':
   key=f'case_{case:02d}';tag=f'pilot_{key}_{mode}_001';cmd=[sys.executable,str(ROOT/'agents/run_trial.py'),'--mode',mode,'--trial-id',tag,'--baseline',str(ROOT/f'results/pilot_admission_001/{key}_123_native'),'--fault-source',str(ROOT/f'private/pilot_opaque_001/{key}.sv'),'--xezim',str(ROOT/'.deps/xezim-observer/target/release/xezim'),'--stimuli',str(ROOT/'stimulus/seed_123'),'--out',str(ROOT/'results'/tag)];r=subprocess.run(cmd,text=True,capture_output=True);(ROOT/'results'/(tag+'_command.log')).write_text(r.stdout+r.stderr)
   tpath=ROOT/'results'/tag/'trial.json';trial=json.loads(tpath.read_text()) if tpath.exists() else {'status':'HOST_FAILURE'}
   grade_cmd=[sys.executable,str(ROOT/'tools/grade_patch.py'),'--trial',str(tpath),'--xezim',str(ROOT/'.deps/xezim-observer/target/release/xezim'),'--fault-source',str(ROOT/f'private/pilot_opaque_001/{key}.sv'),'--out',str(ROOT/'results'/(tag+'_grade'))];g=subprocess.run(grade_cmd,text=True,capture_output=True);(ROOT/'results'/(tag+'_grade_command.log')).write_text(g.stdout+g.stderr);gp=ROOT/'results'/(tag+'_grade')/'grade.json';grade=json.loads(gp.read_text()) if gp.exists() else {'status':'HOST_FAILURE'};entry={'case':key,'mode':mode,'trial_status':trial['status'],'grade':grade['status'],'trial_exit':r.returncode,'grade_exit':g.returncode};f.write(json.dumps(entry)+'\n');f.flush();print(json.dumps(entry),flush=True)
