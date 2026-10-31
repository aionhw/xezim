"""Fresh functional/monitor admission of temporal pilot candidates."""
import json,shutil,subprocess
from pathlib import Path
from run_sim import run
from monitor_oracle import evaluate
ROOT=Path(__file__).resolve().parents[1];private=ROOT/'private/temporal_pilots_001';out=ROOT/'results/temporal_admission_001';out.mkdir(exist_ok=False);catalog=json.loads((private/'catalog_private.json').read_text());binary=ROOT/'.deps/xezim-observer/target/release/xezim';compiler=ROOT/'.deps/iverilog-installed/bin/iverilog';vvp=ROOT/'.deps/iverilog-installed/bin/vvp'
for seed in (123,9876,2026):
 src=ROOT/('stimulus/zero_2026' if seed==2026 else f'stimulus/seed_{seed}');dst=ROOT/f'stimulus/temporal_{seed}';shutil.copytree(src,dst);meta=json.loads((dst/'stimulus.json').read_text());meta['check_monitor_claims']=True;(dst/'stimulus.json').write_text(json.dumps(meta,indent=2)+'\n')
clean={'role':'components','source':str(private/'clean_components.sv'),'tb':str(private/'tb_temporal.sv'),'extra_sources':[str(ROOT/'rtl/tlul_mem_endpoint.sv')],'patch_target':'rtl/tlul_components.sv'};all_cases={'clean':clean,**catalog};summary={}
for key,cfg in all_cases.items():
 summary[key]={'scope':'PILOT_ONLY','role':cfg['role'],'schedules':{}};(private/(key+'_case.json')).write_text(json.dumps(cfg,indent=2)+'\n');exe=out/(key+'.vvp');c=subprocess.run([str(compiler),'-g2012','-s','tb_tlul','-o',str(exe),cfg['source'],*cfg['extra_sources'],cfg['tb']],capture_output=True,text=True,timeout=120);(out/(key+'_compile.log')).write_text(c.stdout+c.stderr)
 for seed in ((123,9876,2026) if key=='clean' else (123,9876)):
  stim=ROOT/f'stimulus/temporal_{seed}';meta=json.loads((stim/'stimulus.json').read_text());x=run(binary,cfg['source'],stim,out/f'{key}_{seed}_research','native' if key!='clean' else 'none',capture_wave=key!='clean',case_config=cfg)
  p=out/f'{key}_{seed}_reference';p.mkdir();shutil.copyfile(stim/'stimulus.hex',p/'stimulus.hex')
  if c.returncode==0:r=subprocess.run([str(vvp),str(exe),'+STIM=stimulus.hex','+NREQ=96',f'+SEED={seed}','+CTRL_OP=0','+CTRL_AT=10','+CTRL_LEN=12'],cwd=p,capture_output=True,text=True,timeout=120);text=r.stdout;code=r.returncode
  else:text='';code=c.returncode
  (p/'stdout.log').write_text(text);o=evaluate(text,meta);(p/'oracle.json').write_text(json.dumps(o,indent=2)+'\n');kinds=sorted({e['kind'] for e in x['checks']['issues']});actual=any(k not in ('incomplete_simulation','incomplete_transactions','insufficient_traffic','missing_or_duplicate_monitor_samples') for k in kinds)
  summary[key]['schedules'][str(seed)]={'research_exit':x['exit_code'],'research_oracle':x['oracle_status'],'reference_exit':code,'reference_oracle':o['status'],'issues':kinds,'symptom':actual,'source':cfg['source']}
 if key=='clean':summary[key]['status']='PASS' if all(r['research_exit']==0 and r['reference_exit']==0 and r['research_oracle']=='PASS' and r['reference_oracle']=='PASS' for r in summary[key]['schedules'].values()) else 'FAIL'
 else:summary[key]['status']='ADMITTED_PILOT_ONLY' if all(r['research_exit']==0 and r['reference_exit']==0 and r['research_oracle']=='FAIL' and r['reference_oracle']=='FAIL' and r['symptom'] for r in summary[key]['schedules'].values()) else 'EXCLUDED_PILOT'
receipt={'status':'CLEAN_CONTROL_PASS' if summary['clean']['status']=='PASS' else 'INVALID_CLEAN_CONTROL_STOP','cases':summary};(out/'receipt_private.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2));raise SystemExit(0 if summary['clean']['status']=='PASS' else 1)
