import sys, tempfile, unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools'))
from control_suite import first_diff

class ControlComparisonTests(unittest.TestCase):
    def test_size_and_ready_are_compared_in_negative_controls(self):
        sample='PIN|cycle=1|av=1|ar=1|aop=4|asz=2|asrc=1|addr=0|mask=15|adata=0|dv=1|dr=1|dop=1|dsz=2|dsrc=1|ddata=0|derr=0\n'
        with tempfile.TemporaryDirectory() as d:
            a=Path(d)/'a'; b=Path(d)/'b';a.write_text(sample)
            for field in ('asz','dsz','dr'):
                b.write_text(sample.replace(field+'=2',field+'=1') if field!='dr' else sample.replace('dr=1','dr=0'))
                self.assertIn(field,first_diff(a,b)['changed_pins'])
