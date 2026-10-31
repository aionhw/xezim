"""Offline scheduling checks; synthetic IDs never represent experimental cases."""
import sys,unittest,tempfile
from pathlib import Path
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'tools'))
from run_primary_schedule import make_plan,continuation_allowed,prepare

class PrimaryScheduleTests(unittest.TestCase):
    def catalog(self):return {'cases':[{'case_id':f'case_{i:016x}','kind':'fault' if i<12 else 'clean'} for i in range(14)]}
    def test_exact_allocation_and_no_duplicate_cells(self):
        rows=make_plan(self.catalog());self.assertEqual(len(rows),168);self.assertEqual(len({r['trial_id'] for r in rows}),168)
        self.assertEqual({(r['case_id'],r['mode'],r['repeat']) for r in rows},{(f'case_{i:016x}',m,r) for i in range(14) for m in 'ABCD' for r in (1,2,3)})
    def test_order_is_deterministic_and_rotates(self):
        catalog=self.catalog();self.assertEqual(make_plan(catalog),make_plan({'cases':list(reversed(catalog['cases']))}))
        rows=make_plan(catalog);orders=[''.join(r['mode'] for r in rows if r['case_id']=='case_0000000000000000' and r['repeat']==rep) for rep in (1,2,3)];self.assertEqual(orders,['ABCD','BCDA','CDAB'])
    def test_only_exact_quota_violation_can_continue(self):
        self.assertTrue(continuation_allowed({'status':'FAILED','error':"AppServerError: capability/model violation: ['case tool action limit exceeded']"}))
        for message in ["AppServerError: capability/model violation: ['case tool action limit exceeded', 'unexpected shell tool']","AppServerError: capability/model violation: ['wrong model']","AppServerError: capability/model violation: not a list",'AppServerError: unauthorized account','ValueError: native qualification mismatch']:
            self.assertFalse(continuation_allowed({'status':'FAILED','error':message}))
    def test_failed_subject_output_is_not_replaced(self):
        self.assertTrue(continuation_allowed({'status':'FAILED','error':'JSONDecodeError: invalid final JSON'}))
        self.assertTrue(continuation_allowed({'status':'FAILED','error':'ValueError: Stage I missing/invalid issued evidence citation(s)'}))
        self.assertFalse(continuation_allowed({'status':'IN_PROGRESS'}))
    def test_missing_review_prevents_capture_and_output(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'results',prefix='OFFLINE_SCHEDULER_') as td:
            dest=Path(td)/'campaign'
            with patch('run_primary_schedule.verify_admission',side_effect=ValueError('independent review missing')),patch('run_primary_schedule.run',side_effect=AssertionError('NO CAPTURE BEFORE REVIEW')):
                with self.assertRaisesRegex(ValueError,'review missing'):prepare(Path(td)/'catalog.json',Path(td)/'admission.json',dest,{})
            self.assertFalse(dest.exists())
    def test_incomplete_catalog_is_not_scheduled(self):
        c=self.catalog();c['cases'].pop()
        with self.assertRaisesRegex(ValueError,'allocation'):make_plan(c)
if __name__=='__main__':unittest.main()
