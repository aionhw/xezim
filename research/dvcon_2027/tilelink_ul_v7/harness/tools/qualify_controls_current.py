import json,subprocess,sys
from pathlib import Path
from control_suite import execute,first_diff
ROOT=Path(__file__).resolve().parents[1];out=ROOT/'results/control_qualification_001';out.mkdir(exist_ok=False);cases={'clean':ROOT/'rtl/tlul_mem_endpoint.sv','fault':ROOT/'private/pilot_opaque_001/case_01.sv'};summary={}
for name,source in cases.items():
 for target,at in [('request_throttle',10),('response_backpressure',25)]:
  key=name+'_'+target;prediction='Engineering prediction: delaying the selected input changes subsequent non-target protocol pins; both negative controls retain the complete baseline pin trace.'
  x=execute(str(ROOT/'.deps/xezim-observer/target/release/xezim'),source,ROOT/'stimulus/seed_123',out/(key+'_research'),target,at,12,prediction)
  cmd=[sys.executable,str(ROOT/'tools/reference_control_suite.py'),'--source',str(source),'--stimuli',str(ROOT/'stimulus/seed_123'),'--out',str(out/(key+'_reference')),'--target',target,'--at-cycle',str(at),'--duration','12','--prediction',prediction];r=subprocess.run(cmd,text=True,capture_output=True,timeout=240);(out/(key+'_reference_command.log')).write_text(r.stdout+r.stderr);y=json.loads((out/(key+'_reference')/'receipt.json').read_text());pairs={}
  for kind in ('baseline','experiment','unrelated','idempotent'):
   def pins(p):return [s for s in p.read_text().splitlines() if s.startswith(('PIN|','SIM_END|','CONTROL|'))]
   pairs[kind]=pins(out/(key+'_research')/kind/'stdout.log')==pins(out/(key+'_reference')/(kind+'.log'))
  summary[key]={'research_status':x['status'],'reference_status':y['status'],'cross_simulator_traces_equal':pairs,'research_oracles':x['independent_oracles'],'first_difference':x['first_non_target_difference']}
receipt={'status':'PASS' if all(x['research_status']=='PASS' and x['reference_status']=='PASS' and all(x['cross_simulator_traces_equal'].values()) for x in summary.values()) else 'FAIL','scope':'engineering predictions; not subject-selected D interventions','cases':summary};(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2));raise SystemExit(0 if receipt['status']=='PASS' else 1)
