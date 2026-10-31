import sys, unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools'))
from generate_stimulus import make_transactions,pack
from check_pins import evaluate

class IndependentOracleTests(unittest.TestCase):
    def test_pack(self):
        t={'op':4,'source':7,'size':2,'address':64,'mask':15,'data':0xABCDEF12}
        w=pack(t)
        self.assertEqual((w>>61)&7,4)
        self.assertEqual((w>>57)&15,7)
        self.assertEqual((w>>38)&65535,64)
        self.assertEqual((w>>2)&0xffffffff,0xABCDEF12)
    def test_source_cycle(self):
        txs=make_transactions(123,96)
        self.assertEqual(len(txs),96)
        self.assertEqual(len(set(t['source'] for t in txs[:16])),16)
        self.assertTrue(any(t['mask']!=15 for t in txs))
        self.assertTrue(any(t['data']==0 for t in txs))
    def test_oracle_clean_and_corruption(self):
        tx={'i':0,'op':4,'source':2,'size':2,'address':0,'mask':15,'data':0}
        prefix='PIN|cycle=1|av=1|ar=1|aop=4|asz=2|asrc=2|addr=0|mask=15|adata=0|dv=0|dr=1|dop=0|dsz=0|dsrc=0|ddata=0|derr=0\n'
        tail='PIN|cycle=2|av=0|ar=0|aop=4|asz=2|asrc=2|addr=0|mask=15|adata=0|dv=1|dr=1|dop=1|dsz=2|dsrc=2|ddata=0|derr=0\nSIM_END|issued=1|retired=1|drained=1|cycle=3\n'
        p=evaluate(prefix+tail,{'transactions':[tx]},min_transactions=1)
        self.assertEqual(p['status'],'PASS',p['issues'])
        bad=evaluate(prefix+tail.replace('ddata=0|derr=0','ddata=5|derr=0'),{'transactions':[tx]},1)
        self.assertEqual(bad['status'],'FAIL')
        self.assertEqual(bad['issues'][0]['kind'],'read_data_mismatch')

if __name__=='__main__': unittest.main()
