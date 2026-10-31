#!/usr/bin/env python3
"""Fresh simulator/oracle engineering admission. No model inference."""
import hashlib,json,re,subprocess,sys
from pathlib import Path
from run_sim import run
from check_pins import evaluate,parse
from generate_stimulus import make_transactions,pack
ROOT=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=ROOT/'results/clean_admission';out.mkdir(exist_ok=False)
 zero=ROOT/'stimulus/zero_2026';zero.mkdir(exist_ok=False);txs=make_transactions(2026,96)
 for t in txs:t['data']=0
 text=''.join(f'{pack(t):016x}\n' for t in txs);(zero/'stimulus.hex').write_text(text);(zero/'stimulus.json').write_text(json.dumps({'seed':2026,'count':96,'transactions':txs,'stimulus_sha256':sha(zero/'stimulus.hex')},indent=2)+'\n')
 binary=ROOT/'.deps/xezim/target/release/xezim';compiler=ROOT/'.deps/iverilog-installed/bin/iverilog';vvp=ROOT/'.deps/iverilog-installed/bin/vvp';source=ROOT/'rtl/tlul_mem_endpoint.sv';tb=ROOT/'rtl/tb_tlul.sv'
 result={'binary_sha256':sha(binary),'compiler_sha256':sha(compiler),'vvp_sha256':sha(vvp),'source_sha256':sha(source),'tb_sha256':sha(tb),'clean':{},'checker_controls':{}}
 compile_cmd=[str(compiler),'-g2012','-s','tb_tlul','-o',str(out/'clean.vvp'),str(source),str(tb)]
 c=subprocess.run(compile_cmd,text=True,capture_output=True,timeout=120);(out/'reference_compile.log').write_text(c.stdout+c.stderr);result['compile']={'argv':compile_cmd,'exit_code':c.returncode}
 if c.returncode:raise RuntimeError('reference compile failed; see immutable log')
 for name in ('seed_123','seed_9876','zero_2026'):
  stim=ROOT/'stimulus'/name;meta=json.loads((stim/'stimulus.json').read_text());x=run(binary,source,stim,out/('xezim_'+name),'none',binary_source_sha='80f00d354fc68c436569b12b6d4e175ef105a652')
  cmd=[str(vvp),str(out/'clean.vvp'),f'+STIM={stim/"stimulus.hex"}','+NREQ=96',f'+SEED={meta["seed"]}','+CTRL_OP=0','+CTRL_AT=10','+CTRL_LEN=12'];r=subprocess.run(cmd,text=True,capture_output=True,timeout=120)
  (out/('reference_'+name+'.log')).write_text(r.stdout);(out/('reference_'+name+'.stderr')).write_text(r.stderr);o=evaluate(r.stdout,meta);(out/('reference_'+name+'_oracle.json')).write_text(json.dumps(o,indent=2)+'\n')
  def pins(text):return [l for l in text.splitlines() if l.startswith(('PIN|','SIM_END|'))]
  match=pins(r.stdout)==pins((out/('xezim_'+name)/'stdout.log').read_text());result['clean'][name]={'xezim_status':x['status'],'reference_status':o['status'],'reference_exit':r.returncode,'reference_argv':cmd,'pin_equality':match,'traffic_qualified':x['traffic_qualified']}
 log=(out/'reference_seed_123.log').read_text();lines=log.splitlines(keepends=True);meta=json.loads((ROOT/'stimulus/seed_123/stimulus.json').read_text());samples=[(i,parse(l)[0][0]) for i,l in enumerate(lines) if l.startswith('PIN|')]
 def replace(line,k,v):return re.sub(r'(?<=\|)'+k+r'=-?\d+',k+'='+str(v),line,count=1)
 for kind in ('wrong_source','wrong_opcode','missing_d','byte_data','stall_payload'):
  rows=lines.copy();changed=False
  for j,(i,s) in enumerate(samples):
   if kind=='stall_payload' and s['dv'] and not s['dr'] and j+1<len(samples):
    k,t=samples[j+1];rows[k]=replace(rows[k],'ddata',t['ddata']^1);changed=True;break
   if s['dv'] and s['dr']:
    field,val=(('dsrc',99) if kind=='wrong_source' else ('dop',s['dop']^1) if kind=='wrong_opcode' else ('dv',0) if kind=='missing_d' else ('ddata',s['ddata']^1) if kind=='byte_data' and s['dop']==1 else (None,None))
    if field:rows[i]=replace(rows[i],field,val);changed=True;break
  if not changed:raise RuntimeError('no valid corruption sample for '+kind)
  corrupted=''.join(rows);(out/('control_'+kind+'.log')).write_text(corrupted);o=evaluate(corrupted,meta);result['checker_controls'][kind]=o
 mutant=out/'byte_mask_mutant.sv';original=source.read_text();needle='if (a_mask[0]) write_word[7:0]';assert original.count(needle)==1;mutant.write_text(original.replace(needle,"if (a_mask[0] && a_opcode != 3'd1) write_word[7:0]"))
 mx=run(binary,mutant,ROOT/'stimulus/seed_123',out/'byte_mask_xezim','none')
 cmd=[str(compiler),'-g2012','-s','tb_tlul','-o',str(out/'mutant.vvp'),str(mutant),str(tb)];c=subprocess.run(cmd,capture_output=True,text=True,timeout=120);(out/'mutant_compile.log').write_text(c.stdout+c.stderr)
 if c.returncode:raise RuntimeError('mutant reference compilation failed')
 cmd=[str(vvp),str(out/'mutant.vvp'),f'+STIM={ROOT/"stimulus/seed_123/stimulus.hex"}','+NREQ=96','+SEED=123','+CTRL_OP=0','+CTRL_AT=10','+CTRL_LEN=12'];r=subprocess.run(cmd,capture_output=True,text=True,timeout=120);(out/'byte_mask_reference.log').write_text(r.stdout);mo=evaluate(r.stdout,meta)
 (out/'byte_mask_reference_oracle.json').write_text(json.dumps(mo,indent=2)+'\n')
 result['byte_mask_mutant']={'xezim_oracle':mx['oracle_status'],'xezim_exit':mx['exit_code'],'reference_oracle':mo['status'],'reference_exit':r.returncode,'source_sha256':sha(mutant),'reference_argv':cmd}
 ok=all(v['xezim_status']=='PASS' and v['reference_status']=='PASS' and v['reference_exit']==0 and v['pin_equality'] for v in result['clean'].values()) and result['clean']['seed_123']['traffic_qualified'] and all(v['status']=='FAIL' for v in result['checker_controls'].values())
 ok=ok and mx['oracle_status']=='FAIL' and mx['exit_code']==0 and mo['status']=='FAIL' and r.returncode==0
 result['status']='PASS' if ok else 'FAIL';(out/'receipt.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':result['status'],'clean':result['clean'],'checker_controls':{k:v['status'] for k,v in result['checker_controls'].items()}},indent=2));return 0 if ok else 1
if __name__=='__main__':sys.exit(main())
