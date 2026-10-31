#!/usr/bin/env python3
"""Matched sampled-field recorder engineering costs, never a model efficacy trial."""
import hashlib,json,os,re,statistics,subprocess,time
from pathlib import Path
from check_pins import evaluate,parse
ROOT=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=ROOT/'results/matched_recorders_001';out.mkdir(exist_ok=False)
 fields={'cycle_count':('dut.cycle_count',32),'next_pick':('dut.next_pick',4),'a_valid':('a_valid',1),'a_ready':('a_ready',1),'a_source':('a_source',4),'a_address':('a_address',16),'d_valid':('d_valid',1),'d_ready':('d_ready',1),'d_source':('d_source',4),'d_data':('d_data',32)}
 tb=(ROOT/'rtl/tb_tlul.sv').read_text();decl='\n'.join(f'reg [{width-1}:0] capture_{name};' for name,(_,width) in fields.items());tb=tb.replace('module tb_tlul;','module tb_tlul;\n'+decl,1)
 tb=tb.replace('$dumpvars(0, tb_tlul);','$dumpvars(0, '+', '.join('capture_'+k for k in fields)+');')
 assignments='\n'.join(f'      capture_{k} = {source};' for k,(source,_) in fields.items());tb=tb.replace('      $dvcon_observe("sample", "tlul");',assignments+'\n      $dvcon_observe("sample", "tlul");')
 tb='\n'.join(l for l in tb.splitlines() if '$dvcon_observe("accepted_' not in l)+'\n';tbpath=out/'tb_tlul_matched.sv';tbpath.write_text(tb);assert all('capture_'+k in tb for k in fields)
 rule=json.loads((ROOT/'protocol/native_recipe_candidate.json').read_text());rule['name']='matched_sampled_fields';rule['trigger']['type']='always';rule['watch']=['capture_'+k for k in fields];rule['capture'].update({'pretrigger_events':0,'posttrigger_events':0,'max_events':8192,'max_bytes':16777216});(out/'rule.json').write_text(json.dumps(rule,indent=2)+'\n')
 binary=ROOT/'.deps/xezim-observer/target/release/xezim';source=ROOT/'rtl/tlul_mem_endpoint.sv';stim=ROOT/'stimulus/seed_123';meta=json.loads((stim/'stimulus.json').read_text());runs=[]
 # Three fresh runs per mode, with order rotated to reduce order dependence.
 for repetition in range(3):
  modes=['none','vcd','value','native'];modes=modes[repetition:]+modes[:repetition]
  for mode in modes:
   p=out/f'r{repetition}_{mode}';p.mkdir();env={k:v for k,v in os.environ.items() if not k.startswith(('XEZIM_','DVCON_'))}
   cmd=[str(binary),'-D','DVCON_NATIVE=1','--simulate','--no-cache','--error-exit','--profile','--max-time','40000ns','-s','tb_tlul',str(source),str(tbpath),f'+STIM={stim/"stimulus.hex"}','+NREQ=96','+SEED=123','+CTRL_OP=0','+CTRL_AT=10','+CTRL_LEN=12']
   if mode=='vcd':cmd[1:1]=['--wave'];cmd.append('+VCD')
   if mode=='value':env['XEZIM_VALUE_TRACE']='capture_'
   if mode=='native':env.update({'DVCON_OBS_RULE':str(out/'rule.json'),'DVCON_OBS_OUT':str(p/'events.jsonl'),'DVCON_OBS_MANIFEST':str(p/'manifest.json'),'DVCON_OBS_STATS':str(p/'stats.json'),'DVCON_RUN_ID':p.name})
   started=time.monotonic();r=subprocess.run(['/usr/bin/time','-v','-o',str(p/'resource.log')]+cmd,cwd=p,env=env,text=True,capture_output=True,timeout=240);wall=time.monotonic()-started;(p/'stdout.log').write_text(r.stdout);(p/'stderr.log').write_text(r.stderr);oracle=evaluate(r.stdout,meta);(p/'oracle.json').write_text(json.dumps(oracle,indent=2)+'\n')
   times=[55000+10000*(s['cycle']-1) for s in parse(r.stdout)[0]];updates=[];raw_bytes=0
   if mode=='native':
    for line in (p/'events.jsonl').read_text().splitlines():
     e=json.loads(line)
     if e.get('event')=='sample':updates.append((e['time_ticks'],e['object'],e['new']))
    raw_bytes=(p/'events.jsonl').stat().st_size
   elif mode=='value':
    trace=[s for s in r.stderr.splitlines() if re.match(r'^\[value-trace\] t=\d+ capture_',s)];raw_bytes=sum(len(s.encode())+1 for s in trace)
    for s in trace:
     m=re.match(r'^\[value-trace\] t=(\d+) (capture_\w+) (.*?) \(',s);assert m,s
     val=m[3].split()[-1];width=fields[m[2][8:]][1]
     if val.startswith("'h"):val=''.join(format(int(c,16),'04b') if c.lower() not in 'xz' else c.lower()*4 for c in val[2:])
     val=val.rjust(width,val[0] if val and val[0].lower() in 'xz' else '0')[-width:];updates.append((int(m[1]),m[2],val.lower()))
   elif mode=='vcd':
    text=(p/'wave.vcd').read_text();raw_bytes=len(text.encode());codes={};tick=0
    for line in text.splitlines():
     parts=line.split()
     if line.startswith('$var'):codes[parts[3]]=(parts[4],int(parts[2]))
     elif line.startswith('#'):tick=int(line[1:])
     elif line and line[0] in 'bB' and len(parts)==2 and parts[1] in codes:
      name,width=codes[parts[1]];v=parts[0][1:].lower();v=v.rjust(width,v[0] if v[0] in 'xz' else '0');updates.append((tick,name,v))
     elif line and line[0].lower() in '01xz' and line[1:] in codes:
      name,width=codes[line[1:]];updates.append((tick,name,line[0].lower()*width))
   normalized=[];current={};cursor=0;updates.sort(key=lambda x:x[0])
   if mode!='none':
    for tick in times:
     while cursor<len(updates) and updates[cursor][0]<=tick:
      _,name,val=updates[cursor];current[name]=val;cursor+=1
     normalized.append({'time_ticks':tick,'fields':{name:current.get(name) for name in ('capture_'+k for k in fields)}})
    (p/'normalized_samples.json').write_text(json.dumps(normalized,separators=(',',':'))+'\n')
   rss=re.search(r'Maximum resident set size \(kbytes\): (\d+)',(p/'resource.log').read_text());phases=[s for s in (r.stdout+'\n'+r.stderr).splitlines() if '[PHASE]' in s]
   row={'repetition':repetition,'mode':mode,'argv':cmd,'exit_code':r.returncode,'oracle':oracle['status'],'wall_seconds':wall,'peak_rss_kib':int(rss[1]) if rss else None,'raw_recorder_bytes':raw_bytes,'phase_records':phases,'normalized_sha256':sha(p/'normalized_samples.json') if normalized else None,'samples':len(times),'normalized_missing_fields':sum(v is None for s in normalized for v in s['fields'].values())};runs.append(row);(p/'receipt.json').write_text(json.dumps(row,indent=2)+'\n')
 hashes={x['normalized_sha256'] for x in runs if x['mode']!='none'};ok=len(hashes)==1 and all(x['exit_code']==0 and x['oracle']=='PASS' and x['normalized_missing_fields']==0 for x in runs)
 summary={'status':'PASS_MATCHED_SAMPLED_COVERAGE' if ok else 'FAIL_COVERAGE_OR_ORACLE','scope':'instrumented synthetic TB snapshot fields at posedge; not scheduler-wide provenance or production benchmark','binary_sha256':sha(binary),'tb_sha256':sha(tbpath),'rule_sha256':sha(out/'rule.json'),'repetitions':3,'normalized_all_recorders_equal':len(hashes)==1,'runs':runs,'costs':{m:{'wall_median_seconds':statistics.median(x['wall_seconds'] for x in runs if x['mode']==m),'peak_rss_median_kib':statistics.median(x['peak_rss_kib'] for x in runs if x['mode']==m),'recorder_bytes':sorted({x['raw_recorder_bytes'] for x in runs if x['mode']==m})} for m in ['none','vcd','value','native']}}
 (out/'receipt.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps({k:v for k,v in summary.items() if k!='runs'},indent=2));return 0 if ok else 1
if __name__=='__main__':raise SystemExit(main())
