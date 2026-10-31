#!/usr/bin/env python3
"""Execute partial native port qualification; never grants comparative C/D admission."""
import hashlib,json,os,subprocess,sys
from pathlib import Path
from check_pins import evaluate,parse
ROOT=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=ROOT/'results/native_restart_probe_001';out.mkdir(exist_ok=False)
 binary=ROOT/'.deps/xezim-observer/target/release/xezim';stock=ROOT/'.deps/xezim/target/release/xezim';recipe=json.loads((ROOT/'protocol/native_recipe_candidate.json').read_text())
 summary={'status':'IN_PROGRESS','binary_sha256':sha(binary),'stock_binary_sha256':sha(stock),'recipe_sha256':sha(ROOT/'protocol/native_recipe_candidate.json'),'scope':'engineering probe only; matched costs and model mapper still required','runs':{}}
 def execute(name,args,rule=None):
  p=out/name;p.mkdir();env={k:v for k,v in os.environ.items() if not k.startswith(('XEZIM_','DVCON_'))}
  if rule is not None:
   (p/'rule.json').write_text(json.dumps(rule,indent=2)+'\n')
   env.update({'DVCON_OBS_RULE':str(p/'rule.json'),'DVCON_OBS_OUT':str(p/'events.jsonl'),'DVCON_OBS_MANIFEST':str(p/'manifest.json'),'DVCON_OBS_STATS':str(p/'stats.json'),'DVCON_RUN_ID':name})
  cmd=[str(binary),'--simulate','--no-cache','--error-exit']+args
  r=subprocess.run(cmd,capture_output=True,text=True,env=env,cwd=p,timeout=240);(p/'stdout.log').write_text(r.stdout);(p/'stderr.log').write_text(r.stderr)
  summary['runs'][name]={'argv':cmd,'exit_code':r.returncode};return r,p
 meta=json.loads((ROOT/'stimulus/seed_123/stimulus.json').read_text());args=['--max-time','40000ns','-s','tb_tlul',str(ROOT/'rtl/tlul_mem_endpoint.sv'),str(ROOT/'rtl/tb_tlul.sv'),f'+STIM={ROOT/"stimulus/seed_123/stimulus.hex"}','+NREQ=96','+SEED=123','+CTRL_OP=0','+CTRL_AT=10','+CTRL_LEN=12']
 r,p=execute('tlul_native',['-D','DVCON_NATIVE=1']+args,recipe);oracle=evaluate(r.stdout,meta);(p/'oracle.json').write_text(json.dumps(oracle,indent=2)+'\n');events=[json.loads(s) for s in (p/'events.jsonl').read_text().splitlines()];stats=json.loads((p/'stats.json').read_text());t=next(i for i,e in enumerate(events) if e.get('event')=='accepted_d');pins,_=parse(r.stdout);pins={x['cycle']:x for x in pins};mapping={'a_valid':'av','a_ready':'ar','a_source':'asrc','a_address':'addr','d_valid':'dv','d_ready':'dr','d_source':'dsrc','d_data':'ddata'};mismatch=[];count=0
 for e in events:
  if e.get('event')!='sample' or e.get('object') not in mapping:continue
  count+=1;c=(e['time_ticks']-55000)//10000+1
  if int(e['new'],2)!=pins[c][mapping[e['object']]]:mismatch.append(e['seq'])
 summary['tlul_checks']={'oracle':oracle['status'],'pretrigger':t,'posttrigger':len(events)-t-1,'matched_pin_fields':count,'mismatches':mismatch,'stats':stats,'provenance_null':all(e.get('writer') is None and e.get('source') is None for e in events),'phase':sorted({e.get('phase') for e in events})}
 def trace(text):return [x for x in text.splitlines() if x.startswith(('PIN|','SIM_END|'))]
 baseline=(ROOT/'results/clean_admission/xezim_seed_123/stdout.log').read_text();summary['native_matches_stock']=trace(r.stdout)==trace(baseline)
 off,op=execute('observer_disabled',args);summary['disabled_matches_stock']=trace(off.stdout)==trace(baseline)
 fixture=out/'fixture.sv';fixture.write_text('''`timescale 1ns/1ps
module native_fixture; reg [3:0] state; initial begin
state=4'b10xz; #1; $dvcon_observe("sample","fixture");
state=4'bzx01; #1; $dvcon_observe("sample","fixture");
state=4'b0101; #1; $dvcon_observe("sample","fixture");
$dvcon_observe("finish","fixture"); $finish; end endmodule
''')
 rule=json.loads(json.dumps(recipe));rule['watch']=['state'];rule['trigger']['type']='always';rule['capture']['max_events']=64;rule['capture']['max_bytes']=8192
 fr,fp=execute('four_state',['-s','native_fixture',str(fixture)],rule);fe=[json.loads(s) for s in (fp/'events.jsonl').read_text().splitlines()];summary['four_state_preserved']=[e['new'] for e in fe]==['10xz','zx01','0101']
 rule['capture']['max_events']=3
 tr,tp=execute('truncation',['-s','native_fixture',str(fixture)],rule);te=[json.loads(s) for s in (tp/'events.jsonl').read_text().splitlines()];summary['truncation_valid']=len(te)==3 and te[-1]['kind']=='truncation' and json.loads((tp/'stats.json').read_text())['truncation_count']==1
 rule['watch']=['nonexistent']
 br,bp=execute('unresolved_watch',['-s','native_fixture',str(fixture)],rule);summary['unresolved_watch_rejected']='unresolved approved object' in br.stdout and not (bp/'events.jsonl').exists()
 ok=oracle['status']=='PASS' and t==64 and len(events)-t-1==32 and not mismatch and count>0 and summary['native_matches_stock'] and summary['disabled_matches_stock'] and summary['four_state_preserved'] and summary['truncation_valid'] and summary['unresolved_watch_rejected']
 summary['status']='PROBES_PASS_NOT_FULLY_QUALIFIED' if ok else 'FAIL';(out/'receipt.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps({k:v for k,v in summary.items() if k!='runs'},indent=2));return 0 if ok else 1
if __name__=='__main__':sys.exit(main())
