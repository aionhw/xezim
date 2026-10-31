#!/usr/bin/env python3
"""Minimal real Codex app-server stdio client (ChatGPT sign-in, no API key).

Only the explicitly documented app-server JSONL/RPC protocol is used. Each
instance launches a NEW process; no thread/resume or thread/fork occurs.
The client does not control provider-side prompt caches.

Docs: https://learn.chatgpt.com/docs/app-server
"""
from __future__ import annotations

import json
import hashlib
import os
import queue
import subprocess
import threading
import time
from pathlib import Path


CASE_QUALIFICATION = Path(__file__).resolve().parents[1]/'results/case_permissions_qualification.json'
TOOL_QUALIFICATION = Path(__file__).resolve().parents[1]/'results/case_tool_allowlist_qualification.json'
CASE_RUNTIME = Path(__file__).resolve().parents[1]/'.deps/codex_cli_strict/runtime'
CASE_CONFIG = {'default_permissions':'dvcon_case', 'permissions.dvcon_case': {'filesystem': {':minimal':'read', ':workspace_roots':'read', str(CASE_RUNTIME):'read'}, 'network': {'enabled':False}}, 'features.shell_tool':False, 'features.unified_exec':False, 'features.multi_agent':False, 'features.memories':False, 'web_search':'disabled', 'plugins':{'github@openai-curated':{'enabled':False}}}
CASE_CONFIG.update({'features.code_mode':{'enabled':False,'direct_only_tool_namespaces':['functions']},
                    'features.code_mode_only':False,
                    'features.code_mode_only_strict_3p_tools':False})


class AppServerError(RuntimeError):
    pass


def is_chatgpt_account(result: dict) -> bool:
    return (result.get('account') or {}).get('type') == 'chatgpt'


def select_model(result: dict, expected: str, effort: str) -> dict:
    models = result.get('data') or []
    for model in models:
        if model.get('model') == expected or model.get('id') == expected:
            efforts = model.get('supportedReasoningEfforts') or []
            supported = {v.get('reasoningEffort') if isinstance(v, dict) else v for v in efforts}
            if effort not in supported:
                raise AppServerError(f'{expected} available but {effort} effort not advertised; do not substitute')
            return {'model': expected, 'effort': effort, 'hidden': bool(model.get('hidden', False))}
    raise AppServerError(f'{expected} not available to this signed-in Codex account; no fallback allowed')


def cached_token_observations(events: list[dict]) -> list[dict]:
    """Preserve available usage records; NEVER infer missing cached tokens as 0."""
    return [e.get('params', {}) for e in events if e.get('method') == 'thread/tokenUsage/updated']


class CodexAppServer:
    def __init__(self, *, codex='codex', stderr_path: Path | None = None,
                 event_path: Path | None = None, workspace: Path | None = None):
        self.codex = codex
        self.workspace = Path(workspace).resolve() if workspace else None
        self.stderr_path = stderr_path
        self.event_path = event_path
        self.proc = None
        self._q = queue.Queue()
        self._next_id = 1
        self._deferred = {}
        self._reader = None
        self._stderr_file = None
        self._event_file = None
        self.handler = None
        self.events = []
        self.violation = []
        self.agent_messages = []
        self.turn_completed = {}
        self.expected_tools = None
        self.server_requests = []

    def __enter__(self):
        if self.workspace is not None and Path(self.codex).name in ('codex', 'codex.exe'):
            if not CASE_QUALIFICATION.is_file() or json.loads(CASE_QUALIFICATION.read_text()).get('status')!='PASS':
                raise AppServerError('case-only model read boundary is unverified; refusing real subject')
            if not TOOL_QUALIFICATION.is_file():
                raise AppServerError('case-only tool exposure is unqualified; refusing real subject')
            tools_q=json.loads(TOOL_QUALIFICATION.read_text())
            if tools_q.get('status')!='PASS' or tools_q.get('codex_sha256')!=hashlib.sha256(Path(self.codex).resolve().read_bytes()).hexdigest():
                raise AppServerError('case-only tool qualification binary mismatch')
            permission_q=json.loads(CASE_QUALIFICATION.read_text())
            files=permission_q.get('runtime_files',{})
            if set(files)!={'bin/codex','codex-resources/bwrap'} or any(
                    not (CASE_RUNTIME/name).is_file() or hashlib.sha256((CASE_RUNTIME/name).read_bytes()).hexdigest()!=digest
                    for name,digest in files.items()):
                raise AppServerError('case-only runtime bundle identity mismatch')
            version=subprocess.check_output([self.codex,'--version'],text=True).strip()
            if json.loads(CASE_QUALIFICATION.read_text()).get('codex_version')!=version:
                raise AppServerError('case-only qualification version mismatch')
            if any(self.workspace.iterdir()):
                raise AppServerError('model workspace must start empty')
        env = os.environ.copy()
        for key in list(env):
            if key.startswith('CODEX_') and key != 'CODEX_HOME':
                env.pop(key, None)
        # An accidental API key environment must never switch the study from Pro.
        for key in ('OPENAI_API_KEY', 'CODEX_API_KEY', 'CODEX_SESSION_ID', 'CODEX_THREAD_ID', 'CODEX_TOOL_CALL_ID', 'CODEX_PERMISSION_PROFILE'):
            env.pop(key, None)
        env['XEZIM_DVCON_CASE_TOOLS_ONLY']='1'
        if self.stderr_path:
            self.stderr_path.parent.mkdir(parents=True, exist_ok=True)
            self._stderr_file = self.stderr_path.open('w', encoding='utf-8')
        if self.event_path:
            self.event_path.parent.mkdir(parents=True, exist_ok=True)
            self._event_file = self.event_path.open('w', encoding='utf-8')
        self.proc = subprocess.Popen(
            [self.codex, 'app-server', '--disable', 'daemon_auto_start', '--disable', 'memories', '--disable', 'workspace_dependencies'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=self._stderr_file or subprocess.DEVNULL, text=True,
            encoding='utf-8', bufsize=1, env=env,
            cwd=str(self.workspace) if self.workspace is not None else None)
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()
        return self

    def _read_loop(self):
        try:
            for line in self.proc.stdout:
                try:
                    obj = json.loads(line)
                    self._q.put(obj)
                except json.JSONDecodeError:
                    self._q.put({'__malformed__': line[:400]})
        finally:
            self._q.put({'__eof__': True})

    def _send(self, message: dict):
        if self.proc is None or self.proc.poll() is not None:
            raise AppServerError('Codex app-server exited before sending request')
        if self._event_file:
            self._event_file.write(json.dumps({'direction':'client_send','message':message},separators=(',',':'))+'\n')
            self._event_file.flush()
        self.proc.stdin.write(json.dumps(message, separators=(',', ':')) + '\n')
        self.proc.stdin.flush()

    def notify(self, method: str, params: dict):
        self._send({'method': method, 'params': params})

    def rpc(self, method: str, params: dict, timeout=30) -> dict:
        request_id = self._next_id
        self._next_id += 1
        self._send({'id': request_id, 'method': method, 'params': params})
        deadline = time.monotonic() + timeout
        while True:
            if request_id in self._deferred:
                response = self._deferred.pop(request_id)
                if 'error' in response:
                    raise AppServerError(f'{method} failed: {response["error"]}')
                return response.get('result') or {}
            self._consume(deadline)

    def _consume(self, deadline: float):
        timeout = deadline - time.monotonic()
        if timeout <= 0:
            raise AppServerError('Timed out waiting for Codex response/turn; no fabricated result')
        try:
            item = self._q.get(timeout=min(timeout, 5))
        except queue.Empty:
            if self.proc is not None and self.proc.poll() is not None:
                raise AppServerError('Codex app-server terminated unexpectedly')
            return
        if '__eof__' in item:
            raise AppServerError('Codex app-server stdout closed unexpectedly')
        if '__malformed__' in item:
            raise AppServerError('Malformed stdout line; no scored trial')
        if 'method' in item:
            if 'id' in item:
                self._server_request(item)
            else:
                self._notification(item)
            return
        if 'id' in item:
            self._deferred[item['id']] = item

    def _notification(self, event: dict):
        method = event.get('method')
        self.events.append(event)
        if self._event_file:
            # Stored only in untracked trial-specific results; may include private source.
            self._event_file.write(json.dumps(event, separators=(',', ':')) + '\n')
            self._event_file.flush()
        if method == 'item/completed':
            item = (event.get('params') or {}).get('item') or {}
            if item.get('type') == 'agentMessage' and item.get('text'):
                self.agent_messages.append(item['text'])
        if method == 'turn/completed':
            info = (event.get('params') or {}).get('turn') or {}
            if info.get('id'):
                self.turn_completed[info['id']] = info
        if method == 'model/rerouted':
            self.violation.append('model rerouted; actual model differs from registered model')
        if method == 'item/started':
            item = (event.get('params') or {}).get('item') or {}
            if item.get('type') in {'commandExecution','fileChange','webSearch','mcpToolCall','collabToolCall','collabAgentToolCall'}:
                self.violation.append('disallowed built-in capability used: '+str(item.get('type')))
        if method in ('warning','configWarning'):
            details = str(event.get('params') or {})
            if any(v in details.lower() for v in ('sandbox disabled','sandbox not available','permission denied to apply sandbox')):
                self.violation.append('sandbox warning: '+details[:400])

    def _server_request(self, request: dict):
        self.server_requests.append(request)
        if self._event_file:
            self._event_file.write(json.dumps({'direction':'server_request',**request},separators=(',',':'))+'\n')
            self._event_file.flush()
        method = request['method']
        if method == 'item/tool/call' and self.handler is not None:
            params = request.get('params') or {}
            try:
                if sum(r.get('method')=='item/tool/call' for r in self.server_requests)>12:
                    self.violation.append('case tool action limit exceeded')
                    raise AppServerError('case tool action limit exceeded')
                if params.get('namespace') not in (None, ''):
                    raise AppServerError('unrecognized dynamic tool namespace')
                val = self.handler(params.get('tool'), params.get('arguments') or {})
                result = {'contentItems': [{'type':'inputText','text':json.dumps(val, sort_keys=True,separators=(',',':'))}],
                          'success': 'error' not in val}
            except Exception as exc:
                result = {'contentItems': [{'type':'inputText','text':json.dumps({'error':type(exc).__name__+': '+str(exc)},separators=(',',':'))}],
                          'success':False}
            self._send({'id':request['id'],'result':result})
            return
        self.violation.append('unexpected server request: '+method)
        # Never grant network/filesystem/command access or reveal extra information.
        self._send({'id':request['id'], 'error':{'code':-32601,'message':'not authorized for this blind study'}})

    def initialize(self):
        self.initialize_result = self.rpc('initialize', {'clientInfo':{'name':'xezim_dvcon_tlul_pro',
            'title':'Xezim DVCon isolated evaluation','version':'0.7.0'},
            'capabilities':{'experimentalApi':True}})
        self.notify('initialized', {})

    def preflight(self, model='gpt-6.1-sol', effort='medium'):
        account = self.rpc('account/read', {'refreshToken':False})
        if not is_chatgpt_account(account):
            raise AppServerError('ChatGPT sign-in required: use `codex login`; API-key auth forbidden')
        selected = select_model(self.rpc('model/list', {'limit':100,'includeHidden':True}), model, effort)
        safe_account = account.get('account') or {}
        return {'auth_type':'chatgpt', 'reported_plan_type':safe_account.get('planType'), **selected}

    def start_thread(self, cwd: Path, model: str, tools: list[dict]):
        # Workspace routing rebuilds retained config without request overrides.
        # Preserve the named selection in config as well as thread/start.
        if CASE_CONFIG.get('default_permissions') != 'dvcon_case':
            raise AppServerError('case permission default must survive config reload')
        self.expected_tools={tool['name'] for tool in tools}
        if not self.expected_tools.issubset({'read_evidence','run_controlled_experiment'}):
            raise AppServerError('unexpected dynamic tool registration')
        result = self.rpc('thread/start', {'model':model,'cwd':str(cwd),
            'approvalPolicy':'never','permissions':'dvcon_case',
            'config':CASE_CONFIG,'allowProviderModelFallback':False,
            'selectedCapabilityRoots':[], 'dynamicTools':tools},timeout=45)
        if result.get('model')!=model or (result.get('activePermissionProfile') or {}).get('id')!='dvcon_case':
            raise AppServerError('model or named permissions profile mismatch')
        if result.get('runtimeWorkspaceRoots') != [str(cwd)]:
            raise AppServerError('unexpected runtime workspace roots')
        sources = result.get('instructionSources')
        if sources is None:
            raise AppServerError('instructionSources not reported; blinding cannot be verified')
        if sources:
            raise AppServerError('unexpected project/user instruction sources: '+str(sources))
        info = result.get('thread') or {}
        if not info.get('id'):
            raise AppServerError('thread/start returned no thread id')
        return info['id']

    def turn(self, *, thread_id, cwd, model, effort, text, timeout=1200):
        # Only trial-specific directory is readable to model-side tools. This
        # is an application sandbox policy, not an independently proven OS cage.
        start_idx = len(self.agent_messages)
        deadline = time.monotonic()+timeout
        result = self.rpc('turn/start', {'threadId':thread_id,
            'input':[{'type':'text','text':text}], 'cwd':str(cwd),
            'approvalPolicy':'never','permissions':'dvcon_case',
            'model':model,'effort':effort}, timeout=min(45,timeout))
        turn_id = (result.get('turn') or {}).get('id')
        if not turn_id:
            raise AppServerError('turn/start returned no turn id')
        while turn_id not in self.turn_completed:
            self._consume(deadline)
        turn = self.turn_completed[turn_id]
        if turn.get('status') != 'completed':
            raise AppServerError(f'turn {turn_id} ended with status {turn.get("status")}: {turn.get("error")}')
        if self.violation:
            raise AppServerError('capability/model violation: '+str(self.violation))
        if Path(self.codex).name in ('codex','codex.exe'):
            self.verify_tool_surface()
        answers = self.agent_messages[start_idx:]
        if not answers:
            raise AppServerError('no completed agent message returned')
        return {'turn_id':turn_id,'text':answers[-1]}

    def verify_tool_surface(self):
        if not self.stderr_path:
            raise AppServerError('tool surface audit log missing')
        if self._stderr_file:self._stderr_file.flush()
        lines=self.stderr_path.read_text().splitlines()
        manifests=[json.loads(line.split('[dvcon-model-tools] ',1)[1])
                   for line in lines if '[dvcon-model-tools] ' in line]
        if not manifests or not any('[dvcon-case-tools]' in line for line in lines):
            raise AppServerError('compiled tool policy evidence missing')
        for specs in manifests:
            names=[]
            for spec in specs:
                if spec.get('type')=='namespace' and spec.get('name')=='functions':
                    children=spec.get('tools',[])
                else:children=[spec]
                for child in children:
                    if child.get('type')!='function':
                        raise AppServerError('unexpected tool kind in model surface')
                    names.append(child.get('name'))
            if set(names)!=self.expected_tools or len(names)!=len(set(names)):
                raise AppServerError('model tool surface differs from case API: '+str(names))
        return {'manifest_count':len(manifests),'allowed_tools':sorted(self.expected_tools)}

    def __exit__(self, *_):
        if self.proc:
            try:
                if self.proc.stdin:
                    self.proc.stdin.close()
            except Exception:
                pass
            if self.proc.poll() is None:
                self.proc.terminate()
                try:self.proc.wait(timeout=4)
                except subprocess.TimeoutExpired:self.proc.kill();self.proc.wait(timeout=3)
        if self.proc and self.proc.stdout:self.proc.stdout.close()
        if self._event_file:self._event_file.close()
        if self._stderr_file:self._stderr_file.close()
        return False
