#!/usr/bin/env python3
"""No-inference app-server probe of a named, case-only permissions profile."""
import argparse,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'agents'))
from codex_app_server import CodexAppServer

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--out',type=Path,required=True);parser.add_argument('--codex-bin',default='codex');args=parser.parse_args()
 out=args.out.resolve();out.mkdir(parents=True,exist_ok=False);workspace=out/'workspace';workspace.mkdir()
 params={'model':'gpt-6.1-sol','cwd':str(workspace),'approvalPolicy':'never','permissions':'dvcon_case','config':{'permissions.dvcon_case':{'filesystem':{':minimal':'read',':workspace_roots':'read'},'network':{'enabled':False}},'features.shell_tool':False,'features.unified_exec':False,'features.multi_agent':False,'web_search':'disabled'},'dynamicTools':[],'allowProviderModelFallback':False}
 (out/'thread_request.json').write_text(json.dumps(params,indent=2)+'\n')
 receipt={'purpose':'configuration probe; no turn/start and no model inference','status':'IN_PROGRESS'}
 try:
  with CodexAppServer(codex=args.codex_bin,stderr_path=out/'stderr.log') as app:
   app.initialize();receipt['initialize']=app.initialize_result;receipt['process_id']=app.proc.pid;receipt['preflight']=app.preflight('gpt-6.1-sol','medium');result=app.rpc('thread/start',params,timeout=60)
   receipt.update({'status':'THREAD_CREATED_NOT_QUALIFIED','response_keys':sorted(result),'activePermissionProfile':result.get('activePermissionProfile'),'runtimeWorkspaceRoots':result.get('runtimeWorkspaceRoots'),'sandbox':result.get('sandbox'),'instructionSources':result.get('instructionSources'),'actual_model':result.get('model'),'thread_id':(result.get('thread') or {}).get('id')})
 except Exception as exc:receipt.update({'status':'FAILED','error':str(exc)})
 (out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2));return 0 if receipt['status']=='THREAD_CREATED_NOT_QUALIFIED' else 1
if __name__=='__main__':sys.exit(main())
