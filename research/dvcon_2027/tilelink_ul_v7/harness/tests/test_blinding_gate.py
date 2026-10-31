import sys, tempfile, unittest
from unittest.mock import patch
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'agents'))
from codex_app_server import CodexAppServer, AppServerError

class BlindingGateTests(unittest.TestCase):
    def test_real_subject_fails_closed_without_verified_boundary(self):
        with tempfile.TemporaryDirectory() as d:
            with patch('codex_app_server.CASE_QUALIFICATION',Path(d)/'missing'), self.assertRaisesRegex(AppServerError,'read boundary is unverified'):
                with CodexAppServer(codex='codex',workspace=Path(d)):
                    self.fail('must not start an unverified real subject')

    def test_missing_retained_profile_default_fails_before_rpc(self):
        from codex_app_server import CASE_CONFIG
        from unittest.mock import Mock
        app=CodexAppServer()
        app.rpc=Mock(side_effect=AssertionError('invalid configuration reached RPC'))
        with patch.dict(CASE_CONFIG, {'default_permissions':None}):
            with self.assertRaisesRegex(AppServerError,'survive config reload'):
                app.start_thread(Path('/unused'), 'gpt-6.1-sol', [])
        app.rpc.assert_not_called()
