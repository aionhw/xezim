import json, sys, tempfile, unittest
from pathlib import Path
from types import SimpleNamespace
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'agents'))
from run_trial import validate_baseline

class ValueAdmissionTests(unittest.TestCase):
    def test_warning_cannot_satisfy_value_gate(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d); (p/'stdout.log').write_text(''); (p/'stderr.log').write_text("[value-trace] pattern 'bad' matched no signal\n")
            (p/'receipt.json').write_text(json.dumps({'actual_value_trace_marker_found':True}))
            args=SimpleNamespace(baseline=p, mode='B')
            with self.assertRaisesRegex(ValueError,'actual value trace records'): validate_baseline(args)
