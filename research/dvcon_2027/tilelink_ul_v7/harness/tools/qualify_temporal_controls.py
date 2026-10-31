"""Engineering controls for the separately admitted temporal-component fixture."""
import json,subprocess,sys
from pathlib import Path
from control_suite import execute
ROOT=Path(__file__).resolve().parents[1];out=ROOT/'results/temporal_control_qualification_001';out.mkdir(exist_ok=False);rows=[]
for key in ('clean','temporal_03','temporal_05'):
 cfgp=ROOT/f'private/temporal_pilots_001/{key}_case.json';cfg=json.loads(cfgp.read_text())
 for target in ('request_throttle','response_backpressure'):
  tag=key+'_'+target;prediction='Engineering qualification: the active input changes later accepted traffic or public monitor claims; inert and idempotent controls preserve all samples.'
  r=execute(ROOT/'.deps/xezim-observer/target/release/xezim',cfg['source'],ROOT/'stimulus/temporal_123',out/(tag+'_research'),target,23,8,prediction,case_config=cfg)
  ref=out/(tag+'_reference');cmd=[sys.executable,str(ROOT/'tools/reference_control_suite.py'),'--source',cfg['source'],'--stimuli',str(ROOT/'stimulus/temporal_123'),'--out',str(ref),'--target',target,'--at-cycle','23','--duration','8','--prediction',prediction,'--case-manifest',str(cfgp)];p=subprocess.run(cmd,capture_output=True,text=True);(out/(tag+'_command.log')).write_text(p.stdout+p.stderr);rr=json.loads((ref/'receipt.json').read_text());same={}
  for name in ('baseline','experiment','unrelated','idempotent'):
   lines=lambda p:[s for s in p.read_text().splitlines() if s.startswith(('PIN|','MON|','CONTROL|','SIM_END|'))]
   same[name]=lines(out/(tag+'_research')/name/'stdout.log')==lines(ref/(name+'.log'))
  rows.append({'case':key,'target':target,'research_status':r['status'],'reference_status':rr['status'],'all_public_observations_match':same,'research_oracles':r['independent_oracles']})
receipt={'status':'PASS' if all(r['research_status']==r['reference_status']=='PASS' and all(r['all_public_observations_match'].values()) for r in rows) else 'FAIL','scope':'Engineering prediction; no model subjects or semantic prediction grades','cases':rows};(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2))
