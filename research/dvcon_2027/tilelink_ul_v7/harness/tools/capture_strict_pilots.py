"""Create fresh dependency-bound native/value/VCD pilot bundles, without inference."""
import json
from pathlib import Path
from run_sim import run
ROOT=Path(__file__).resolve().parents[1]
out=ROOT/'results/strict_pilot_capture_001';out.mkdir(exist_ok=False);rows=[]
for kind,count in [('named',5),('temporal',8)]:
 for i in range(1,count+1):
  cfg=json.loads((ROOT/f'private/temporal_pilots_001/temporal_{i:02d}_case.json').read_text()) if kind=='temporal' else {}
  src=cfg.get('source',str(ROOT/f'private/pilot_opaque_001/case_{i:02d}.sv'));stim=ROOT/('stimulus/temporal_123' if cfg else 'stimulus/seed_123')
  r=run(ROOT/'.deps/xezim-observer/target/release/xezim',src,stim,out/f'{kind}_{i:02d}','native',capture_wave=True,case_config=cfg)
  rows.append({'case':f'{kind}_{i:02d}','oracle':r['oracle_status'],'exit':r['exit_code'],'native':r['native_observer_executed'],'value':r['actual_value_trace_marker_found'],'wave':r['actual_vcd_file_found']})
receipt={'status':'PASS' if all(r['oracle']=='FAIL' and r['exit']==0 and r['native'] and r['value'] and r['wave'] for r in rows) else 'FAIL','cases':rows}
(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2))
