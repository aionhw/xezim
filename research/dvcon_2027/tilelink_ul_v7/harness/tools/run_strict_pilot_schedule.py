"""Uniform fresh development pilot schedule after strict tool qualification.

Includes every named and temporal pilot; failed outcomes are never replaced.
"""
import json,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def main():
 q=ROOT/'results/case_tool_allowlist_qualification.json'
 if not q.is_file() or json.loads(q.read_text()).get('status')!='PASS':raise SystemExit('strict tool qualification is required before any subject')
 ledger=ROOT/'results/strict_pilot_schedule_001.jsonl'
 with ledger.open('x') as f:
  cases=[]
  for i in range(1,6):
   cases.append((f'named_{i:02d}',ROOT/f'private/pilot_opaque_001/case_{i:02d}.sv',ROOT/f'results/strict_pilot_capture_001/named_{i:02d}',ROOT/'stimulus/seed_123',None))
  for i in range(1,9):
   cfgpath=ROOT/f'private/temporal_pilots_001/temporal_{i:02d}_case.json';cfg=json.loads(cfgpath.read_text())
   cases.append((f'temporal_{i:02d}',Path(cfg['source']),ROOT/f'results/strict_pilot_capture_001/temporal_{i:02d}',ROOT/'stimulus/temporal_123',cfgpath))
  plan=[{'case':key,'mode':mode,'repeat':1,'scope':'PILOT_ONLY'} for key,*_ in cases for mode in 'ABCD']
  with (ROOT/'results/strict_pilot_schedule_plan_001.json').open('x') as pf:json.dump(plan,pf,indent=2);pf.write('\n')
  for key,source,baseline,stim,cfgpath in cases:
   for mode in 'ABCD':
    tag=f'strict_pilot_{key}_{mode}_001';trial=ROOT/'results'/tag
    cmd=[sys.executable,str(ROOT/'agents/run_trial.py'),'--mode',mode,'--trial-id',tag,'--baseline',str(baseline),'--fault-source',str(source),'--xezim',str(ROOT/'.deps/xezim-observer/target/release/xezim'),'--stimuli',str(stim),'--out',str(trial)]
    if cfgpath:cmd+=['--case-manifest',str(cfgpath)]
    r=subprocess.run(cmd,text=True,capture_output=True);(ROOT/'results'/(tag+'_command.log')).write_text(r.stdout+r.stderr)
    tp=trial/'trial.json';d=json.loads(tp.read_text()) if tp.is_file() else {'status':'HOST_FAILURE'}
    gc=[sys.executable,str(ROOT/'tools/grade_patch.py'),'--trial',str(tp),'--xezim',str(ROOT/'.deps/xezim-observer/target/release/xezim'),'--fault-source',str(source),'--out',str(ROOT/'results'/(tag+'_grade'))]
    g=subprocess.run(gc,text=True,capture_output=True);(ROOT/'results'/(tag+'_grade_command.log')).write_text(g.stdout+g.stderr);gp=ROOT/'results'/(tag+'_grade')/'grade.json';gd=json.loads(gp.read_text()) if gp.is_file() else {'status':'HOST_FAILURE'}
    entry={'case':key,'mode':mode,'trial_status':d['status'],'grade':gd['status'],'trial_exit':r.returncode,'grade_exit':g.returncode,'error':d.get('error')};f.write(json.dumps(entry)+'\n');f.flush();print(json.dumps(entry),flush=True)
    # Actual identity, exposure or blinding failures stop further inference.
    err=d.get('error','').lower()
    if d['status']=='HOST_FAILURE' or any(word in err for word in ('capability','qualification','model mismatch','sign-in required','not available','tool surface','tool policy','instruction','permissions profile','workspace roots','workspace must','runtime bundle','binary mismatch','case bundle','freeze mismatch','unverified')):
     raise SystemExit('integrity gate failed; preserve failed scheduled subject and stop')
if __name__=='__main__':main()
