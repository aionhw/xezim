#!/usr/bin/env python3
"""Pilot admission only; no primary holdout construction."""
import json,subprocess,shutil
from pathlib import Path
from run_sim import run
from check_pins import evaluate
ROOT=Path(__file__).resolve().parents[1];out=ROOT/'results/pilot_admission_001';out.mkdir(exist_ok=False);private=ROOT/'private/pilot_opaque_001';private.mkdir();catalog=json.loads((ROOT/'private/pilots_exposed_001/manifest_private.json').read_text());result={};binary=ROOT/'.deps/xezim-observer/target/release/xezim';compiler=ROOT/'.deps/iverilog-installed/bin/iverilog';vvp=ROOT/'.deps/iverilog-installed/bin/vvp'
for i,(name,item) in enumerate(catalog.items(),1):
 key=f'case_{i:02d}';source=private/(key+'.sv');shutil.copyfile(item['path'],source);result[key]={'pilot_family':name,'schedules':{},'eligibility':'PENDING'}
 cmd=[str(compiler),'-g2012','-s','tb_tlul','-o',str(out/(key+'.vvp')),str(source),str(ROOT/'rtl/tb_tlul.sv')];c=subprocess.run(cmd,capture_output=True,text=True,timeout=120);(out/(key+'_compile.log')).write_text(c.stdout+c.stderr)
 for seed in (123,9876):
  stim=ROOT/f'stimulus/seed_{seed}';meta=json.loads((stim/'stimulus.json').read_text());x=run(binary,source,stim,out/f'{key}_{seed}_native','native',capture_wave=True)
  p=out/f'{key}_{seed}_reference';p.mkdir();shutil.copyfile(stim/'stimulus.hex',p/'stimulus.hex');r=subprocess.run([str(vvp),str(out/(key+'.vvp')),'+STIM=stimulus.hex','+NREQ=96',f'+SEED={seed}','+CTRL_OP=0','+CTRL_AT=10','+CTRL_LEN=12'],cwd=p,capture_output=True,text=True,timeout=120);(p/'stdout.log').write_text(r.stdout);(p/'stderr.log').write_text(r.stderr);o=evaluate(r.stdout,meta);(p/'oracle.json').write_text(json.dumps(o,indent=2)+'\n');kinds=sorted({e['kind'] for e in x['checks']['issues']});symptom=any(k not in ('incomplete_simulation','incomplete_transactions','insufficient_traffic') for k in kinds);result[key]['schedules'][str(seed)]={'research_exit':x['exit_code'],'research_oracle':x['oracle_status'],'reference_exit':r.returncode,'reference_oracle':o['status'],'functional_or_protocol_symptom':symptom,'issues':kinds,'native_files':x['native_observer_executed'],'value_records':x['actual_value_trace_marker_found'],'vcd':x['actual_vcd_file_found']}
 result[key]['eligibility']='ADMITTED_PILOT_ONLY' if all(v['research_exit']==0 and v['reference_exit']==0 and v['research_oracle']=='FAIL' and v['reference_oracle']=='FAIL' and v['functional_or_protocol_symptom'] and v['native_files'] and v['value_records'] and v['vcd'] for v in result[key]['schedules'].values()) else 'EXCLUDED_PILOT'
(out/'receipt_private.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
