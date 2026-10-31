#!/usr/bin/env python3
"""No-inference filesystem/network negative controls for the case profile."""
import argparse,json,subprocess
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--codex-bin',required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--runtime-root',type=Path);a=p.parse_args();out=a.out.resolve();out.mkdir(exist_ok=False);w=out/'workspace';w.mkdir();(w/'allowed.txt').write_text('CASE_ONLY');(out/'outside.txt').write_text('HOST_SENTINEL');(w/'outside_link').symlink_to(out/'outside.txt')
code='''from pathlib import Path
import json,socket
r={"allowed":Path("allowed.txt").read_text()=="CASE_ONLY"}
for name,path in [("outside_denied",Path("../outside.txt")),("symlink_denied",Path("outside_link"))]:
 try:path.read_text();r[name]=False
 except (PermissionError,FileNotFoundError):r[name]=True
try:Path("new.txt").write_text("bad");r["write_denied"]=False
except OSError:r["write_denied"]=True
try:socket.socket(socket.AF_INET,socket.SOCK_STREAM);r["network_socket_denied"]=False
except OSError:r["network_socket_denied"]=True
print(json.dumps(r))
'''
entries=['":minimal"="read"','":workspace_roots"="read"']
if a.runtime_root:entries.append(json.dumps(str(a.runtime_root.resolve()))+'="read"')
profile='{"filesystem"={'+','.join(entries)+'},"network"={"enabled"=false}}';cmd=[a.codex_bin,'sandbox','-P','dvcon_case','-c','permissions.dvcon_case='+profile,'-C',str(w),'--','/usr/bin/python3','-c',code];r=subprocess.run(cmd,text=True,capture_output=True,timeout=60);receipt={'argv':cmd,'exit_code':r.returncode,'stdout':r.stdout,'stderr':r.stderr,'cli_version':subprocess.check_output([a.codex_bin,'--version'],text=True).strip()}
try:receipt['checks']=json.loads(r.stdout);receipt['status']='PASS' if r.returncode==0 and all(receipt['checks'].values()) else 'FAIL'
except ValueError:receipt['status']='FAIL'
(out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2));raise SystemExit(0 if receipt['status']=='PASS' else 1)
