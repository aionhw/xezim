#!/usr/bin/env python3
"""Read-only authenticated Codex Pro preflight; does not launch model inference."""
import argparse, json, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'agents'))
from codex_app_server import CodexAppServer, AppServerError


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--codex-bin',default=str(ROOT/json.loads((ROOT/'config/model.json').read_text())['codex_cli_path']))
    args=parser.parse_args()
    conf=json.loads((ROOT/'config/model.json').read_text())
    try:
        with CodexAppServer(codex=args.codex_bin) as client:
            client.initialize()
            info=client.preflight(conf['model'],conf['reasoning_effort'])
    except (AppServerError, FileNotFoundError, OSError) as e:
        print('PRO SIGN-IN PREFLIGHT FAILED:',e,file=sys.stderr)
        return 1
    print(json.dumps({'status':'PASS','service':'ChatGPT Codex sign-in',**info},indent=2))
    return 0
if __name__=='__main__':sys.exit(main())
