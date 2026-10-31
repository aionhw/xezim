#!/usr/bin/env python3
"""Run an isolated real Xezim simulation; classify results with independent oracle.

Never treats a simulator exit code or UVM summary as the independent oracle.
Never treats a host-correlated observation as the native observer.
"""
import argparse, hashlib, json, os, re, shutil, subprocess, sys, time
from pathlib import Path
from monitor_oracle import evaluate
from semantic_adapter import summarize

ROOT=Path(__file__).resolve().parents[1]
MODES=('none','vcd','value','external','native')

def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def run(xezim, source, stimuli, out, mode, ctrl_op=0, ctrl_at=10,
        ctrl_len=12, timeout=240, binary_source_sha=None, capture_wave=False, case_config=None):
    xezim=Path(xezim).resolve(); source=Path(source).resolve()
    stimuli=Path(stimuli).resolve(); out=Path(out).resolve()
    meta=json.loads((stimuli/'stimulus.json').read_text())
    if not xezim.is_file(): raise FileNotFoundError('Xezim binary: '+str(xezim))
    if not source.is_file(): raise FileNotFoundError('RTL: '+str(source))
    if out.exists(): raise FileExistsError('create-once run directory: '+str(out))
    if mode not in MODES: raise ValueError('unsupported mode')
    if ctrl_op not in (0,1,2,3,4): raise ValueError('illegal control')
    if not (4 <= ctrl_at <= 2000 and 1<=ctrl_len<=32):
        raise ValueError('control time/duration outside safe envelope')
    out.mkdir(parents=True)
    # SV packed plusarg buffer is 128 bytes: use a short local, hashed copy.
    shutil.copyfile(stimuli/'stimulus.hex',out/'stimulus.hex')
    case_config=case_config or {}
    tb=Path(case_config.get('tb',ROOT/'rtl/tb_tlul.sv')).resolve()
    script=[str(xezim),'--simulate','--no-cache','--error-exit',
            '--max-time','40000ns','-s','tb_tlul',
            str(source),*[str(Path(p).resolve()) for p in case_config.get('extra_sources',[])],str(tb),
            '+STIM=stimulus.hex',f'+NREQ={meta["count"]}',
            f'+SEED={meta["seed"]}',f'+CTRL_OP={ctrl_op}',
            f'+CTRL_AT={ctrl_at}',f'+CTRL_LEN={ctrl_len}']
    env={k:v for k,v in os.environ.items() if not k.startswith('XEZIM_')}
    env['XEZIM_NO_NATIVE_CACHE']='1'
    if mode in ('value','external','native'):
        # Existing tool. Verify actual availability/output on this exact binary.
        env['XEZIM_VALUE_TRACE']='pending,next_pick'
    if mode=='native':
        qualification=ROOT/'results/native_qualification.json'
        if not qualification.is_file():
            raise RuntimeError('native research port has no P2 qualification receipt')
        q=json.loads(qualification.read_text())
        if q.get('status')!='PASS' or q.get('binary_sha256')!=sha(xezim) or q.get('recipe_sha256')!=sha(ROOT/'protocol/native_recipe_candidate.json'):
            raise RuntimeError('native qualification is absent, failed, or does not match binary and recipe')
        # These outputs must be genuine files emitted by the ported research binary.
        env['DVCON_OBS_RULE']=str(ROOT/'protocol/native_recipe_candidate.json')
        env['DVCON_OBS_OUT']=str(out/'native_events.jsonl')
        env['DVCON_OBS_MANIFEST']=str(out/'native_manifest.json')
        env['DVCON_OBS_STATS']=str(out/'native_stats.json')
        env['DVCON_RUN_ID']=out.name.replace(' ','_')[:90]
        script[1:1]=['-D','DVCON_NATIVE=1']
    if mode=='vcd' or capture_wave:
        script[1:1]=['--wave']
        script.append('+VCD')
        # Source-driven VCD must actually be enabled/validated before scoring.
        env['TLUL_WAVEFILE']=str(out/'wave.vcd')
    began=time.monotonic()
    timed_out=False
    try:
        done=subprocess.run(script,cwd=out,env=env,text=True,capture_output=True,timeout=timeout)
        code=done.returncode; stdout=done.stdout; stderr=done.stderr
    except subprocess.TimeoutExpired as exc:
        timed_out=True; code=None
        stdout=(exc.stdout or b'').decode(errors='replace') if isinstance(exc.stdout,bytes) else exc.stdout or ''
        stderr=(exc.stderr or b'').decode(errors='replace') if isinstance(exc.stderr,bytes) else exc.stderr or ''
    wall=time.monotonic()-began
    (out/'stdout.log').write_text(stdout)
    (out/'stderr.log').write_text(stderr)
    oracle=evaluate(stdout,meta)
    (out/'oracle.json').write_text(json.dumps(oracle,indent=2)+'\n')
    if mode=='external':
        (out/'semantic_external.json').write_text(json.dumps(summarize(stdout,512),indent=2)+'\n')
    # Tool availability can differ across builds. Never claim value trace from env alone.
    value_found=bool(re.search(r'(?m)^\[value-trace\] t=\d+ ', stderr))
    wave_path=out/'wave.vcd'
    traffic_qualified=(oracle['a_accepted']>=50 and oracle['d_accepted']>=50 and
        oracle['max_outstanding']>=4 and oracle['distinct_reused_source_ids']>=2 and
        oracle['partial_writes']>=4 and oracle['zero_data_writes']>=2 and
        oracle['d_stall_samples']>0 and oracle['cross_source_completion_inversions']>0)
    result={"status":"PASS" if oracle['status']=='PASS' and code==0 and not timed_out else 'FAIL',
            "mode":mode,"oracle_status":oracle['status'],"exit_code":code,
            "timed_out":timed_out,"wall_seconds":wall,"traffic_qualified":traffic_qualified,"source_sha256":sha(source),
            "binary_sha256":sha(xezim),"expected_xezim_commit":binary_source_sha,
            "stimulus_sha256":sha(stimuli/'stimulus.hex'),"tb_sha256":sha(tb),"case_config":case_config,
            "extra_source_sha256":{str(Path(p).resolve()):sha(p) for p in case_config.get('extra_sources',[])},
            "actual_value_trace_marker_found":value_found,
            "actual_vcd_file_found":wave_path.is_file(),
            "native_observer_executed":(mode=='native' and
                (out/'native_events.jsonl').is_file() and
                (out/'native_manifest.json').is_file() and
                (out/'native_stats.json').is_file()),
            "ctrl_op":ctrl_op,"ctrl_at":ctrl_at,"ctrl_len":ctrl_len,
            "argv":script,"checks":oracle,"output_bytes":len(stdout.encode()),
            "stderr_bytes":len(stderr.encode())}
    if mode in ('value','external','native') and not value_found:
        result['status']='FAIL'
    if capture_wave and not wave_path.is_file():
        result['status']='FAIL'
    if mode=='native' and not result['native_observer_executed']:
        result['status']='FAIL'
    (out/'receipt.json').write_text(json.dumps(result,indent=2)+'\n')
    return result

if __name__=='__main__':
    p=argparse.ArgumentParser()
    p.add_argument('--xezim',type=Path,required=True)
    p.add_argument('--source',type=Path,default=ROOT/'rtl/tlul_mem_endpoint.sv')
    p.add_argument('--stimuli',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--mode',choices=MODES,default='none')
    p.add_argument('--ctrl-op',type=int,default=0)
    p.add_argument('--ctrl-at',type=int,default=10)
    p.add_argument('--ctrl-len',type=int,default=12)
    p.add_argument('--timeout',type=int,default=240)
    a=p.parse_args()
    res=run(a.xezim,a.source,a.stimuli,a.out,a.mode,a.ctrl_op,a.ctrl_at,a.ctrl_len,a.timeout)
    print(json.dumps({k:res[k] for k in ['status','oracle_status','mode','exit_code','timed_out','wall_seconds']},indent=2))
    sys.exit(0 if res['status']=='PASS' else 1)
