import json,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'agents'))
from codex_app_server import CodexAppServer,AppServerError
class ToolSurfaceTests(unittest.TestCase):
 def check(self,specs):
  with tempfile.TemporaryDirectory() as d:
   log=Path(d)/'stderr.log';log.write_text('[dvcon-case-tools] allowed=read_evidence,run_controlled_experiment\n[dvcon-model-tools] '+json.dumps(specs)+'\n')
   app=CodexAppServer(stderr_path=log);app.expected_tools={'read_evidence'};return app.verify_tool_surface()
 def test_only_registered_dynamic_function_passes(self):
  self.assertEqual(self.check([{'type':'function','name':'read_evidence'}])['allowed_tools'],['read_evidence'])
 def test_unexpected_builtin_or_namespace_fails(self):
  for specs in ([{'type':'function','name':'read_evidence'},{'type':'function','name':'apply_patch'}],[{'type':'namespace','name':'web','tools':[{'type':'function','name':'run'}]}],[]):
   with self.subTest(specs=specs),self.assertRaises(AppServerError):self.check(specs)
 def test_missing_compiled_manifest_fails(self):
  with tempfile.TemporaryDirectory() as d:
   log=Path(d)/'stderr.log';log.write_text('normal stock runner log\n');app=CodexAppServer(stderr_path=log)
   with self.assertRaisesRegex(AppServerError,'policy evidence missing'):app.verify_tool_surface()
