"""Sequential pilot-only execution. No retries or outcome replacement."""
import subprocess,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
for mode in ('B','C','D'):
 tag=f'pilot_case01_{mode}_001';cmd=[sys.executable,str(ROOT/'agents/run_trial.py'),'--mode',mode,'--trial-id',tag,'--baseline',str(ROOT/'results/pilot_admission_001/case_01_123_native'),'--fault-source',str(ROOT/'private/pilot_opaque_001/case_01.sv'),'--xezim',str(ROOT/'.deps/xezim-observer/target/release/xezim'),'--stimuli',str(ROOT/'stimulus/seed_123'),'--out',str(ROOT/'results'/tag)];r=subprocess.run(cmd,text=True,capture_output=True);(ROOT/'results'/(tag+'_command.log')).write_text(r.stdout+r.stderr)
 print(mode,'trial exit',r.returncode,flush=True)
 cmd=[sys.executable,str(ROOT/'tools/grade_patch.py'),'--trial',str(ROOT/'results'/tag/'trial.json'),'--xezim',str(ROOT/'.deps/xezim-observer/target/release/xezim'),'--fault-source',str(ROOT/'private/pilot_opaque_001/case_01.sv'),'--out',str(ROOT/'results'/(tag+'_grade'))];g=subprocess.run(cmd,text=True,capture_output=True);(ROOT/'results'/(tag+'_grade_command.log')).write_text(g.stdout+g.stderr);print(mode,'grade exit',g.returncode,flush=True)
