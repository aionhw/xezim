#!/usr/bin/env python3
"""Explicit external TileLink-UL A/D relationship adapter. Not a native plugin.

Derives only information from observed handshakes. It does NOT synthesize
writer/source provenance or original causal timing. Unknowns remain null.
"""
import argparse, json
from pathlib import Path
from check_pins import parse

def summarize(log: str, max_events: int=128):
    samples,_=parse(log)
    open_requests={}; events=[]; missing=[]; total=0
    for s in samples:
        if not all(isinstance(v,int) for v in s.values()): continue
        if s.get('av')==1 and s.get('ar')==1:
            total+=1
            source=s['asrc']
            if source in open_requests: missing.append({'cycle':s['cycle'],'reason':'source_reused','source':source})
            record={'kind':'accepted_a','cycle':s['cycle'],'source':source,
                    'opcode':s['aop'],'address':s['addr'],'size':s['asz'],
                    'mask':s['mask'],'request_ordinal':total,'writer':None,'phase':'posedge_pin_sample'}
            open_requests[source]=record
            events.append(record)
        if s.get('dv')==1 and s.get('dr')==1:
            source=s['dsrc']; match=open_requests.pop(source,None)
            events.append({'kind':'accepted_d','cycle':s['cycle'],'source':source,
                           'opcode':s['dop'],'data':s['ddata'],
                           'matched_a_cycle':match['cycle'] if match else None,
                           'matched_request_ordinal':match['request_ordinal'] if match else None,
                           'unknown_reason':None if match else 'no_matching_accepted_a',
                           'writer':None,'phase':'posedge_pin_sample'})
    clipped=events[:max_events]
    return {'schema':'TLUL_EXTERNAL_PIN_JOIN_V1','type':'external_declarative_mapping',
            'not_native':True,'not_causal_provenance':True,
            'source_identity':'accepted_pin_a_source/d_source',
            'record_count':len(events),'returned_records':len(clipped),
            'truncated':len(clipped)<len(events),
            'unmatched_responses':sum(e.get('unknown_reason') is not None for e in events),
            'unmatched_requests':len(open_requests),'errors':missing[:30],
            'events':clipped}

if __name__=='__main__':
    p=argparse.ArgumentParser()
    p.add_argument('--log',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--max-events',type=int,default=128)
    a=p.parse_args()
    a.out.write_text(json.dumps(summarize(a.log.read_text(errors='replace'),a.max_events),indent=2)+'\n')
    print(a.out)
