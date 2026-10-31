#!/usr/bin/env python3
"""Independent A/D accepted-pin oracle for the declared TileLink-UL subset.

Never reads design internals, expected testbench messages or fault definitions.
Checks the actual accepted transfers against a separate byte-memory reference.
"""
import argparse, collections, json, re, sys
from pathlib import Path

FIELDS = ['cycle','av','ar','aop','asz','asrc','addr','mask','adata',
          'dv','dr','dop','dsz','dsrc','ddata','derr']


def parse(text):
    samples=[]; endings=[]
    for line in text.splitlines():
        if line.startswith('PIN|'):
            obj={}
            for field in line.split('|')[1:]:
                if '=' not in field: continue
                k,v=field.split('=',1)
                try: obj[k]=int(v,10)
                except ValueError: obj[k]=v
            samples.append(obj)
        elif line.startswith('SIM_END|'):
            endings.append(line)
    return samples,endings


def evaluate(log: str, manifest: dict, min_transactions: int=16):
    samples,endings=parse(log)
    issues=[]; memory=collections.defaultdict(int); inflight={}; outstanding_max=0
    count_a=count_d=partial=stalled_d=source_reuses=zero_data_writes=0
    completions=[]; source_history=set(); reused_sources=set(); prior_a=None; prior_d=None
    txs=manifest['transactions']; cursor=0
    for samp in samples:
        c=samp.get('cycle','?')
        for key in FIELDS:
            if key not in samp or not isinstance(samp[key],int):
                issues.append({'cycle':c, 'kind':'noninteger_or_missing_pin','pin':key})
        if any(k not in samp or not isinstance(samp[k],int) for k in FIELDS):
            continue
        s=samp
        a_tuple=(s['aop'],s['asz'],s['asrc'],s['addr'],s['mask'],s['adata'])
        d_tuple=(s['dop'],s['dsz'],s['dsrc'],s['ddata'],s['derr'])
        if prior_a is not None and (s['av']!=1 or a_tuple != prior_a):
            issues.append({'cycle':c,'kind':'a_stall_payload_instability'})
        if prior_d is not None and (s['dv']!=1 or d_tuple != prior_d):
            issues.append({'cycle':c,'kind':'d_stall_payload_instability'})
        prior_a = a_tuple if s['av'] and not s['ar'] else None
        prior_d = d_tuple if s['dv'] and not s['dr'] else None
        if s['av'] and s['ar']:
            count_a+=1
            if cursor >= len(txs):
                issues.append({'cycle':c,'kind':'unexpected_extra_a'})
                continue
            expected=txs[cursor]
            observed=dict(zip(('op','size','source','address','mask','data'), a_tuple))
            if any(observed[k]!=expected[k] for k in observed):
                issues.append({'cycle':c,'kind':'a_intent_mismatch','request':cursor,
                               'observed':observed, 'expected':{k:expected[k] for k in observed}})
            cursor+=1
            source=s['asrc']; addr=s['addr']; op=s['aop']; mask=s['mask']; data=s['adata']
            if source in inflight:
                issues.append({'cycle':c,'kind':'source_reused_while_pending','source':source})
            if source in source_history:
                source_reuses+=1
                reused_sources.add(source)
            source_history.add(source)
            if op not in (0,1,4) or s['asz']!=2 or addr%4 or addr>=1024 or mask>15:
                issues.append({'cycle':c,'kind':'invalid_profile_request'})
            if op in (0,4) and mask!=15:
                issues.append({'cycle':c,'kind':'invalid_full_word_mask'})
            if op == 1 and mask==0:
                issues.append({'cycle':c,'kind':'empty_partial_mask'})
            if op!=4 and data==0 and mask!=0: zero_data_writes+=1
            if op==4:
                resp=memory[addr]
            else:
                w=memory[addr]
                for lane in range(4):
                    if mask & (1<<lane):
                        byte=(data>>(8*lane)) & 255
                        w=(w & ~(255<<(8*lane))) | (byte<<(8*lane))
                memory[addr]=w
                resp=0
                if op==1: partial+=1
            inflight[source]={'op':1 if op==4 else 0, 'data':resp,
                              'size':s['asz'], 'request':cursor-1,'cycle':c}
            outstanding_max=max(outstanding_max,len(inflight))
        if s['dv'] and not s['dr']:
            stalled_d+=1
        if s['dv'] and s['dr']:
            count_d+=1
            source=s['dsrc']
            if source not in inflight:
                issues.append({'cycle':c,'kind':'orphan_d_response','source':source})
            else:
                want=inflight.pop(source)
                completions.append(want['request'])
                if s['dop'] != want['op'] or s['dsz'] != want['size'] or s['derr'] != 0:
                    issues.append({'cycle':c,'kind':'d_metadata_mismatch','source':source,
                                   'expected_op':want['op'],'observed_op':s['dop']})
                if want['op']==1 and s['ddata'] != want['data']:
                    issues.append({'cycle':c,'kind':'read_data_mismatch','source':source,
                                   'expected_data':want['data'],'observed_data':s['ddata'],
                                   'request':want['request']})
    if len(endings)!=1 or 'drained=1' not in endings[0]:
        issues.append({'kind':'incomplete_simulation','end_markers':endings})
    if cursor!=len(txs) or count_d!=len(txs) or inflight:
        issues.append({'kind':'incomplete_transactions','a':count_a,'d':count_d,
                       'expected':len(txs),'pending':sorted(inflight)})
    if count_a < min_transactions: issues.append({'kind':'insufficient_traffic','a':count_a})
    inversions=sum(completions[i]>completions[j]
                   for i in range(len(completions)) for j in range(i+1,len(completions)))
    return {'status':'PASS' if not issues else 'FAIL','sample_count':len(samples),
            'a_accepted':count_a,'d_accepted':count_d,'partial_writes':partial,
            'max_outstanding':outstanding_max,'d_stall_samples':stalled_d,
            'source_reuses_after_retirement':source_reuses,
            'distinct_reused_source_ids':len(reused_sources),
            'zero_data_writes':zero_data_writes,
            'cross_source_completion_inversions':inversions,'issues':issues[:150],
            'total_issue_count':len(issues)}

if __name__=='__main__':
    p=argparse.ArgumentParser()
    p.add_argument('--log',type=Path,required=True)
    p.add_argument('--stimulus',type=Path,required=True)
    p.add_argument('--out',type=Path)
    args=p.parse_args()
    result=evaluate(args.log.read_text(errors='replace'),json.loads(args.stimulus.read_text()))
    serialized=json.dumps(result,indent=2)+'\n'
    if args.out: args.out.write_text(serialized)
    print(serialized)
    sys.exit(0 if result['status']=='PASS' else 1)
