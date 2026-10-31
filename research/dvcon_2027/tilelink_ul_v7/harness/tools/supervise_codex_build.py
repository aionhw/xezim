"""Detached local build supervisor that always saves the observed exit status."""
import datetime,json,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];tag=sys.argv[1];log=ROOT/'results'/(tag+'.log');receipt=ROOT/'results'/(tag+'_exit.json')
if log.exists() or receipt.exists():raise FileExistsError('create-once build attempt')
start=datetime.datetime.now(datetime.timezone.utc).isoformat();r=subprocess.run(['bash',str(ROOT/'tools/build_codex_strict.sh'),str(log),*sys.argv[2:]],cwd=ROOT)
receipt.write_text(json.dumps({'started_utc':start,'ended_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'exit_code':r.returncode,'log':str(log.relative_to(ROOT)),'binary_exists':(ROOT/'.deps/codex_strict_target/release/codex').is_file()},indent=2)+'\n')
