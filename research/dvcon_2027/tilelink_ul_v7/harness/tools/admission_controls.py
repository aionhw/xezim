#!/usr/bin/env python3
"""Exercise pin oracle against independently corrupted real reference trace."""
import json, re, sys
from pathlib import Path
from check_pins import evaluate, parse
root=Path(__file__).resolve().parents[1]
log=(root/'results/admission_20261030/reference.log').read_text()
meta=json.loads((root/'stimulus/seed_123/stimulus.json').read_text())
lines=log.splitlines(keepends=True)
def change(line,key,val): return re.sub(r'(?<=\|)'+key+r'=-?\d+',key+'='+str(val),line,count=1)
def make(kind):
    out=lines.copy(); samples=[(i,parse(l)[0][0]) for i,l in enumerate(lines) if l.startswith('PIN|')]
    if kind=='stall_payload':
        for j,(i,s) in enumerate(samples[:-1]):
            if s['dv'] and not s['dr']:
                k,t=samples[j+1]; out[k]=change(out[k],'ddata',t['ddata']^1); return ''.join(out)
    for i,s in samples:
        if kind=='wrong_source' and s['dv'] and s['dr']:
            out[i]=change(out[i],'dsrc',99); break
        if kind=='wrong_opcode' and s['dv'] and s['dr']:
            out[i]=change(out[i],'dop',s['dop']^1); break
        if kind=='missing_d' and s['dv'] and s['dr']:
            out[i]=change(out[i],'dv',0); break
        if kind=='byte_mask' and s['av'] and s['ar'] and s['aop']==1:
            out[i]=change(out[i],'mask',s['mask']^1); break
    else: raise RuntimeError('no suitable sample: '+kind)
    return ''.join(out)
results={}
for kind in ('wrong_source','wrong_opcode','missing_d','byte_mask','stall_payload'):
    mutated=make(kind); (root/f'results/admission_20261030/control_{kind}.log').write_text(mutated)
    result=evaluate(mutated,meta); results[kind]=result
    print(kind,result['status'],[x['kind'] for x in result['issues'][:4]])
(root/'results/admission_20261030/checker_controls.json').write_text(json.dumps(results,indent=2)+'\n')
sys.exit(0 if all(v['status']=='FAIL' for v in results.values()) else 1)
