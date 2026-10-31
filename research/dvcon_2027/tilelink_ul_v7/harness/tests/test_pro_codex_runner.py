import json
import hashlib
import os
import stat
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'agents'))
from codex_app_server import AppServerError, CodexAppServer, is_chatgpt_account, select_model, cached_token_observations
from run_trial import Host, run_subject, parse_json_answer


class ProCodexContractTests(unittest.TestCase):
    def test_account_auth_never_accepts_api_key(self):
        self.assertTrue(is_chatgpt_account({'account':{'type':'chatgpt','planType':'pro'}}))
        self.assertFalse(is_chatgpt_account({'account':{'type':'apiKey'}}))
        self.assertFalse(is_chatgpt_account({'account':None}))

    def test_no_unavailable_model_or_effort_fallback(self):
        result={'data':[{'id':'gpt-6.1-terra','supportedReasoningEfforts':[{'reasoningEffort':'high'}]}]}
        with self.assertRaises(AppServerError):select_model(result,'gpt-6.1-sol','medium')
        with self.assertRaises(AppServerError):select_model({'data':[{'model':'gpt-6.1-sol','supportedReasoningEfforts':[{'reasoningEffort':'low'}]}]},'gpt-6.1-sol','medium')

    def test_cached_tokens_reported_without_zero_requirement(self):
        events=[{'method':'thread/tokenUsage/updated','params':{'tokenUsage':{'cachedInputTokens':350}}}]
        self.assertEqual(cached_token_observations(events)[0]['tokenUsage']['cachedInputTokens'],350)

    def test_unexpected_builtin_tool_is_violation(self):
        app=CodexAppServer()
        app._notification({'method':'item/started','params':{'item':{'type':'commandExecution'}}})
        self.assertTrue(app.violation)

    @patch('run_trial.verify_freeze',new=lambda **kwargs: None)
    @patch('run_trial.pilot_phase_closed',new=lambda root: False)
    def test_mock_appserver_stages_source_seal_and_cache(self):
        # EXPLICIT OFFLINE TEST MOCK: results must never be scored as agent inference.
        with tempfile.TemporaryDirectory() as td:
            base=Path(td)
            baseline=base/'baseline';baseline.mkdir()
            (baseline/'receipt.json').write_text(json.dumps({'mode':'none','actual_value_trace_marker_found':False}))
            (baseline/'stdout.log').write_text('PIN|cycle=0|av=1\nPIN|cycle=1|av=0\n')
            (baseline/'oracle.json').write_text(json.dumps({'status':'FAIL','issues':[{'kind':'bad_data','cycle':1}]}))
            source=base/'fault.sv';source.write_text('module sample; // SECRET_SOURCE_MARKER\nendmodule\n')
            stimuli=base/'stimuli';stimuli.mkdir();(stimuli/'stimulus.json').write_text('{}')
            (stimuli/'stimulus.hex').write_text('00\n')
            fake_binary=base/'no-simulator';fake_binary.write_text('test transport only')
            (baseline/'wave.vcd').write_text('$scope module tb_tlul $end\n$var reg 1 ! a_valid $end\n$upscope $end\n$enddefinitions $end\n#0\n0!\n')
            receipt=json.loads((baseline/'receipt.json').read_text())
            for key,path in [('source_sha256',source),('binary_sha256',fake_binary),('stimulus_sha256',stimuli/'stimulus.hex'),('tb_sha256',ROOT/'rtl/tb_tlul.sv')]:receipt[key]=hashlib.sha256(path.read_bytes()).hexdigest()
            (baseline/'receipt.json').write_text(json.dumps(receipt))
            mock=ROOT/'tests/mock_codex_app_server.py'
            executable=base/'fake-codex'
            executable.write_text('#!/bin/sh\nexec "'+sys.executable+'" "'+str(mock)+'"\n')
            executable.chmod(executable.stat().st_mode|stat.S_IEXEC)
            out=base/'trial'
            args=types.SimpleNamespace(mode='A',baseline=baseline,fault_source=source,
                stimuli=stimuli,xezim=base/'no-simulator',out=out,trial_id='TEST_UNSCORED',
                engineering_external=False,timeout=20)
            with patch.dict(os.environ,{'MOCK_TRIAL_OUT':str(out),
                                    'OPENAI_API_KEY':'MUST_NOT_PASS_THROUGH',
                                    'CODEX_API_KEY':'MUST_NOT_PASS_THROUGH'}):
                rec=run_subject(args,codex=str(executable))
            self.assertEqual(rec['status'],'UNSCORED_TEST_TRANSPORT',rec.get('error'))
            self.assertEqual(rec['actual_query_types'],['pin'])
            self.assertEqual(len(rec['cache_observations']),2)
            self.assertEqual(rec['cache_observations'][0]['tokenUsage']['cachedInputTokens'],111)
            self.assertTrue((out/'stage1_sealed.json').exists())
            self.assertNotIn('HIDDEN_EMAIL',(out/'trial.json').read_text())
            self.assertNotIn('MUST_NOT_PASS_THROUGH',(out/'raw/codex_events.jsonl').read_text())

    @patch('run_trial.verify_freeze',new=lambda **kwargs: None)
    @patch('run_trial.pilot_phase_closed',new=lambda root: False)
    def test_fabricated_citation_never_reveals_source(self):
        # EXPLICIT OFFLINE TEST MOCK: results must never be scored as agent inference.
        with tempfile.TemporaryDirectory() as td:
            base=Path(td)
            baseline=base/'baseline';baseline.mkdir()
            (baseline/'receipt.json').write_text(json.dumps({'mode':'none','actual_value_trace_marker_found':False}))
            (baseline/'stdout.log').write_text('PIN|cycle=0|av=1\nPIN|cycle=1|av=0\n')
            (baseline/'oracle.json').write_text(json.dumps({'status':'FAIL','issues':[{'kind':'bad_data','cycle':1}]}))
            source=base/'fault.sv';source.write_text('module sample; // SECRET_SOURCE_MARKER\nendmodule\n')
            stimuli=base/'stimuli';stimuli.mkdir();(stimuli/'stimulus.json').write_text('{}')
            (stimuli/'stimulus.hex').write_text('00\n')
            fake_binary=base/'no-simulator';fake_binary.write_text('test transport only')
            (baseline/'wave.vcd').write_text('$scope module tb_tlul $end\n$var reg 1 ! a_valid $end\n$upscope $end\n$enddefinitions $end\n#0\n0!\n')
            receipt=json.loads((baseline/'receipt.json').read_text())
            for key,path in [('source_sha256',source),('binary_sha256',fake_binary),('stimulus_sha256',stimuli/'stimulus.hex'),('tb_sha256',ROOT/'rtl/tb_tlul.sv')]:receipt[key]=hashlib.sha256(path.read_bytes()).hexdigest()
            (baseline/'receipt.json').write_text(json.dumps(receipt))
            mock=ROOT/'tests/mock_codex_app_server.py'
            executable=base/'fake-codex'
            executable.write_text('#!/bin/sh\nexec "'+sys.executable+'" "'+str(mock)+'"\n')
            executable.chmod(executable.stat().st_mode|stat.S_IEXEC)
            out=base/'trial'
            args=types.SimpleNamespace(mode='A',baseline=baseline,fault_source=source,
                stimuli=stimuli,xezim=base/'no-simulator',out=out,trial_id='TEST_UNSCORED',
                engineering_external=False,timeout=20)
            with patch.dict(os.environ,{'MOCK_TRIAL_OUT':str(out),'MOCK_BAD_CITATION':'1',
                                    'OPENAI_API_KEY':'MUST_NOT_PASS_THROUGH',
                                    'CODEX_API_KEY':'MUST_NOT_PASS_THROUGH'}):
                rec=run_subject(args,codex=str(executable))
            self.assertEqual(rec['status'],'FAILED')
            self.assertIn('citation',rec['error'])
            self.assertFalse((out/'stage1_sealed.json').exists())
            self.assertNotIn('II',rec['stages'])

if __name__=='__main__':unittest.main()

class BudgetBriefingTests(unittest.TestCase):
    def test_numeric_bounds_and_budget_are_in_model_visible_prose(self):
        from types import SimpleNamespace
        from run_trial import stage1_instructions
        text=stage1_instructions(SimpleNamespace(mode='D',case_manifest=None,timeout=1200),{'status':'PASS'})
        for required in ('12 tool calls','131072 returned evidence','1200 seconds','1 through 40','4 through 2000','1 through 32','Invalid requests count','insufficient','Required Stage I use'):
            self.assertIn(required,text)
