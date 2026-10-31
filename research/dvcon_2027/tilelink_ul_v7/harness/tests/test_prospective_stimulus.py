"""Common prospective traffic and reproducibility, not fault admission tests."""
import json,sys,tempfile,unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'tools'))
from generate_prospective_stimulus import make,write

class ProspectiveStimulusTests(unittest.TestCase):
    def test_common_traffic_witnesses_without_a_case_parameter(self):
        txs=make(123456,192);writes=[t for t in txs if t['op']!=4]
        self.assertEqual(len(txs),192)
        self.assertTrue(any(a['data']==b['data'] and a['address']!=b['address'] for a,b in zip(writes,writes[1:])))
        self.assertTrue(any(t['op']==1 and t['mask'] in (5,6,9,10) for t in txs))
        self.assertGreaterEqual(sum(t['data']==0 and t['op']!=4 and t['mask'] for t in txs),2)
        self.assertTrue(all(t['size']==2 and t['address']%4==0 and 0<=t['address']<1024 and 0<t['mask']<16 and t['op'] in (0,1,4) for t in txs))
    def test_seed_reproduction_and_create_once(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'results',prefix='OFFLINE_TRAFFIC_') as td:
            base=Path(td);write(9921,192,base/'a');write(9921,192,base/'b');write(9922,192,base/'c')
            self.assertEqual((base/'a/stimulus.hex').read_bytes(),(base/'b/stimulus.hex').read_bytes());self.assertNotEqual((base/'a/stimulus.hex').read_bytes(),(base/'c/stimulus.hex').read_bytes())
            with self.assertRaises(FileExistsError):write(9921,192,base/'a')
    def test_boundary_counts_and_disclosed_profile(self):
        for n in (96,512):self.assertEqual(len(make(321,n)),n)
        for n in (95,513):
            with self.assertRaises(ValueError):make(321,n)
if __name__=='__main__':unittest.main()
