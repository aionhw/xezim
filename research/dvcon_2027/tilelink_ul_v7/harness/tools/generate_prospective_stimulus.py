#!/usr/bin/env python3
"""Common legal temporal traffic; identical recipe for every prospective case.

Directed memory phases witness repeated payloads at different addresses,
read/write/read lifetimes and legal noncontiguous masks. No fault definition,
case ID, expected repair or model response affects transaction generation.
"""
import argparse,hashlib,json,random
from pathlib import Path
from generate_stimulus import make_transactions,pack

def make(seed,count):
    if type(count) is not int or not 96<=count<=512:raise ValueError('count must be96..512')
    rng=random.Random(seed);txs=[]
    first=(0x9137a5c3^(seed&0xffff))&0xffffffff
    second=(first^0x5a36d1e7)&0xffffffff
    masks=(5,10,9,6,3,12,1,8)
    for phase in range(6):
        for word in range(16):
            index=len(txs);op=4 if phase%2 else (1 if phase==4 else 0)
            data=first if phase==0 else (0 if phase==2 and word%5==0 else second if phase==2 else rng.getrandbits(32))
            txs.append({'i':index,'op':op,'source':(index*7+seed%16)%16,'size':2,'address':word*4,'mask':masks[word%len(masks)] if op==1 else 15,'data':data})
    if count>96:
        for t in make_transactions(seed^0x6d3a21,count-96):
            t=dict(t);t['i']=len(txs);t['source']=(t['i']*7+seed%16)%16;txs.append(t)
    return txs

def write(seed,count,out):
    out=Path(out);out.mkdir(parents=True,exist_ok=False);txs=make(seed,count);hexfile=out/'stimulus.hex';hexfile.write_text(''.join(f'{pack(t):016x}\n' for t in txs));meta={'format':'TLUL64_VECTOR_V1','profile':'temporal_reuse_v2','seed':seed,'count':count,'stimulus_sha256':hashlib.sha256(hexfile.read_bytes()).hexdigest(),'check_monitor_claims':True,'transactions':txs};(out/'stimulus.json').write_text(json.dumps(meta,indent=2)+'\n');return meta
if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--seed',type=int,required=True);p.add_argument('--count',type=int,default=192);p.add_argument('--out',type=Path,required=True);a=p.parse_args();m=write(a.seed,a.count,a.out);print(json.dumps({k:v for k,v in m.items() if k!='transactions'},indent=2))
