#!/usr/bin/env python3
"""EXPOSED CALIBRATION ONLY. These source mutations can NEVER become holdouts.

Build after baseline admission, run each fault on >=2 independent schedules,
and record negative/wrong-oracle results, not merely runtime exceptions.
"""
import argparse, hashlib, json, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
SOURCE=ROOT/'rtl/tlul_mem_endpoint.sv'
FAULTS={
 'partial_wrong_lane':(
  'if (a_mask[0]) write_word[7:0]   = a_data[7:0];',
  'if (a_mask[1]) write_word[7:0]   = a_data[7:0];'),
 'd_held_release':(
  'if (d_valid && d_ready) begin',
  'if (d_valid) begin'),
 'wrong_response_word':(
  'response_data[a_source] <= memory[word_index];',
  'response_data[a_source] <= memory[(word_index + 1) % WORDS];'),
 'pending_wrong_retirement':(
  'pending[d_source] <= 0;',
  'pending[(d_source+1)%SOURCES] <= 0;'),
 'stale_d_opcode':(
  "response_opcode[a_source] <= (a_opcode == 3'd4) ? 3'd1 : 3'd0;",
  "response_opcode[a_source] <= (a_opcode == 3'd4 && a_source[0]==0) ? 3'd1 : 3'd0;"),
}

def main():
 p=argparse.ArgumentParser()
 p.add_argument('--out',type=Path,required=True)
 a=p.parse_args();out=a.out.resolve()
 if out.exists():sys.exit('create-once; do not overwrite a pilot')
 out.mkdir(parents=True)
 original=SOURCE.read_text(); manifest={}
 for name,(old,new) in FAULTS.items():
  if original.count(old)!=1:sys.exit(f'anchor mismatch {name}')
  text=original.replace(old,new,1)
  file=out/(name+'.sv');file.write_text(text)
  manifest[name]={'source_sha256':hashlib.sha256(text.encode()).hexdigest(),
                  'eligibility':'pilot_only_not_holdout','path':str(file)}
 (out/'manifest_private.json').write_text(json.dumps(manifest,indent=2)+'\n')
 print('Created pilot-only mutants:',list(manifest))
if __name__=='__main__':main()
