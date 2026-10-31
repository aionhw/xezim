#!/usr/bin/env python3
"""Run an engineering control suite on a separately built Icarus reference."""
import argparse, hashlib, json, subprocess, sys, shutil
from pathlib import Path
from monitor_oracle import evaluate
from control_suite import first_diff, TARGETS
ROOT=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--stimuli',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--target',choices=TARGETS,required=True);p.add_argument('--at-cycle',type=int,required=True);p.add_argument('--duration',type=int,required=True);p.add_argument('--prediction',required=True);p.add_argument('--case-manifest',type=Path);a=p.parse_args()
 if not 4<=a.at_cycle<=2000 or not 1<=a.duration<=32 or len(a.prediction.strip())<12: p.error('invalid target timing or prediction')
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=False);stim=a.stimuli.resolve();source=a.source.resolve()
 (out/'prediction.json').write_text(json.dumps({'target':a.target,'at_cycle':a.at_cycle,'duration':a.duration,'prediction':a.prediction,'origin':'engineering_harness'},indent=2)+'\n')
 cfg=json.loads(a.case_manifest.read_text()) if a.case_manifest else {}
 shutil.copyfile(stim/'stimulus.hex',out/'stimulus.hex')
 compiler=ROOT/'.deps/iverilog-installed/bin/iverilog';vvp=ROOT/'.deps/iverilog-installed/bin/vvp';tb=Path(cfg.get('tb',ROOT/'rtl/tb_tlul.sv'));exe=out/'sim.vvp'; meta=json.loads((stim/'stimulus.json').read_text())
 compile_cmd=[str(compiler),'-g2012','-s','tb_tlul','-o',str(exe),str(source),*cfg.get('extra_sources',[]),str(tb)]
 c=subprocess.run(compile_cmd,text=True,capture_output=True,timeout=120);(out/'compile.log').write_text(c.stdout+c.stderr)
 result={'compile_argv':compile_cmd,'compile_exit':c.returncode,'source_sha256':sha(source),'tb_sha256':sha(tb),'stimulus_sha256':sha(stim/'stimulus.hex'),'compiler_sha256':sha(compiler),'vvp_sha256':sha(vvp),'extra_source_sha256':{p:sha(p) for p in cfg.get('extra_sources',[])},'runs':{},'prediction_origin':'engineering_harness'}
 if c.returncode!=0:
  result['status']='FAIL';(out/'receipt.json').write_text(json.dumps(result,indent=2)+'\n');return 1
 for name,op in [('baseline',0),('experiment',TARGETS[a.target]),('unrelated',2),('idempotent',3)]:
  cmd=[str(vvp),str(exe),'+STIM=stimulus.hex',f'+NREQ={meta["count"]}',f'+SEED={meta["seed"]}',f'+CTRL_OP={op}',f'+CTRL_AT={a.at_cycle}',f'+CTRL_LEN={a.duration}']
  try: r=subprocess.run(cmd,cwd=out,text=True,capture_output=True,timeout=120);code=r.returncode;stdout=r.stdout;stderr=r.stderr;timeout=False
  except subprocess.TimeoutExpired: code=None;stdout='';stderr='timeout';timeout=True
  (out/f'{name}.log').write_text(stdout);(out/f'{name}.stderr').write_text(stderr);oracle=evaluate(stdout,meta)
  (out/f'{name}_oracle.json').write_text(json.dumps(oracle,indent=2)+'\n')
  result['runs'][name]={'argv':cmd,'exit_code':code,'timed_out':timeout,'oracle_status':oracle['status'],'force_release_markers':('CONTROL|phase=force' in stdout and 'CONTROL|phase=release' in stdout) if op else None}
 skip=('dr',) if a.target=='response_backpressure' else ()
 diffs={name:first_diff(out/'baseline.log',out/f'{name}.log',skip if name=='experiment' else ()) for name in ('experiment','unrelated','idempotent')}
 ok=all(v['exit_code']==0 and not v['timed_out'] for v in result['runs'].values()) and all(result['runs'][k]['force_release_markers'] for k in ('experiment','unrelated','idempotent')) and diffs['experiment'] is not None and diffs['unrelated'] is None and diffs['idempotent'] is None
 result.update({'status':'PASS' if ok else 'FAIL','first_differences':diffs,'prediction_judged_by_expert':False,'scope':'engineering only; no model subject'})
 (out/'receipt.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':result['status'],'first_differences':diffs,'oracles':{k:v['oracle_status'] for k,v in result['runs'].items()}},indent=2));return 0 if ok else 1
if __name__=='__main__':sys.exit(main())
