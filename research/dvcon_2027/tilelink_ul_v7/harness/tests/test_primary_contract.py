"""Offline boundary tests; fixture labels are NOT novel hardware faults or review."""
import copy,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'tools'));sys.path.insert(0,str(ROOT/'agents'))
from primary_contract import SCHEMA,digest,validate_preregistration,validate_taxonomy,inside,verify_admission,concrete_symptom,reserve_subject
from generate_holdouts import generate

class PrimaryContractTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='OFFLINE_TEST_ONLY_',dir=ROOT/'results');self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        for folder in ['protocol','tools','config','private/design']:(self.root/folder).mkdir(parents=True,exist_ok=True)
        for name,text in [('protocol/STAGE1_RUBRIC.md','OFFLINE RUBRIC'),('tools/generate_holdouts.py','OFFLINE GENERATOR'),('private/design/clean.txt','OFFLINE_TEST_ONLY token={{SALT}} CLEAN_ANCHOR'),('private/design/tb.txt','OFFLINE_TEST_ONLY NO HARDWARE')]:
            (self.root/name).write_text(text)
        def ref(name):return {'path':name,'sha256':digest(self.root/name)}
        self.tax={'schema':SCHEMA,'status':'FINAL_DESIGN','author_id':'OFFLINE_TEST_CONSTRUCTOR','identifiability_policy':'OFFLINE ONLY','transactions_per_schedule':96,'cases':[]}
        for i in range(14):
            c={'spec_id':f'test_{i}','kind':'fault' if i<12 else 'clean','component':['endpoint','monitor','scoreboard'][i%3],'patch_target':'rtl/tlul_mem_endpoint.sv' if i%3==0 else 'rtl/tlul_components.sv','mechanism_family':f'OFFLINE_FAKE_FAMILY_{i}','mechanism_private':'NO HARDWARE MECHANISM','expected_symptom':'NO RUNTIME CLAIM','identifiability':'OFFLINE ONLY','novelty_review_questions':'NO NOVELTY CLAIM','clean_template':ref('private/design/clean.txt'),'testbench':ref('private/design/tb.txt'),'extra_sources':[],'parameters':{'SALT':{'min':1,'max':65535}},'edit':{'old':'CLEAN_ANCHOR','new':'OFFLINE_MUTATED_ANCHOR'} if i<12 else None};self.tax['cases'].append(c)
        (self.root/'private/design/taxonomy.json').write_text(json.dumps(self.tax))
        self.protocol={'schema':SCHEMA,'status':'FINAL_PROTOCOL','rubric':ref('protocol/STAGE1_RUBRIC.md'),'holdout_generator':ref('tools/generate_holdouts.py'),'taxonomy':ref('private/design/taxonomy.json'),'author_id':'OFFLINE_TEST_CONSTRUCTOR','identifiability_policy':'OFFLINE ONLY','independent_review_plan':'NO ACTUAL REVIEW','prior_family_exclusion_policy':'OFFLINE ONLY','analysis_seed':71032026,'bootstrap_resamples':10000,'allocation':{'faults':12,'clean_controls':2,'modes':list('ABCD'),'repeats':3,'sessions':168},'limits':{'tool_calls':12,'evidence_bytes':131072,'trial_seconds':1200},'model':'gpt-6.1-sol','reasoning_effort':'medium'}
        inputs=validate_taxonomy(self.root,self.tax)
        self.protocol['protected_inputs']={**inputs,**{self.protocol[k]['path']:self.protocol[k]['sha256'] for k in ['rubric','taxonomy','holdout_generator']}}
        (self.root/'config/preregistration.json').write_text(json.dumps(self.protocol));(self.root/'config/model.json').write_text(json.dumps({'model':'gpt-6.1-sol','reasoning_effort':'medium'}))

    def test_hash_presence_does_not_qualify_a_modified_taxonomy(self):
        validate_preregistration(self.root,self.protocol)
        with (self.root/'private/design/taxonomy.json').open('a') as stream:stream.write(' ')
        with self.assertRaisesRegex(ValueError,'hash mismatch'):validate_preregistration(self.root,self.protocol)

    def test_private_templates_are_required_in_protected_inventory(self):
        del self.protocol['protected_inputs']['private/design/clean.txt']
        with self.assertRaisesRegex(ValueError,'inventory'):validate_preregistration(self.root,self.protocol)

    def test_analysis_settings_must_match_implementation(self):
        self.protocol['bootstrap_resamples']=100
        with self.assertRaisesRegex(ValueError,'analysis settings'):validate_preregistration(self.root,self.protocol)

    def test_twelve_distinct_families_and_three_categories_required(self):
        bad=copy.deepcopy(self.tax);bad['cases'][1]['mechanism_family']=bad['cases'][0]['mechanism_family']
        with self.assertRaisesRegex(ValueError,'distinct'):validate_taxonomy(self.root,bad)
        bad=copy.deepcopy(self.tax)
        for c in bad['cases']:c['component']='endpoint'
        with self.assertRaisesRegex(ValueError,'three component'):validate_taxonomy(self.root,bad)

    def test_outside_and_symlink_templates_rejected(self):
        with self.assertRaisesRegex(ValueError,'relative'):inside(self.root,'../outside')
        (self.root/'private/link').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(ValueError,'escapes'):inside(self.root,'private/link')

    def test_generation_is_create_once_and_does_not_admit(self):
        # Explicit offline text fixtures, not RTL instances or experimental evidence.
        (self.root/'config/freeze_receipt.json').write_text('OFFLINE_TEST_ONLY FAKE FREEZE')
        frozen={'protected_inputs':self.protocol['protected_inputs']};dest=self.root/'private/OFFLINE_GENERATED'
        catalog=generate(self.root,dest,frozen,b'\x01'*32)
        self.assertEqual(catalog['status'],'GENERATED_NOT_ADMITTED');self.assertEqual(len(catalog['cases']),14)
        self.assertEqual(sum(c['kind']=='fault' for c in catalog['cases']),12)
        self.assertFalse((dest/'admission.json').exists())
        with self.assertRaises(FileExistsError):generate(self.root,dest,frozen,b'\x02'*32)
        for c in catalog['cases']:
            self.assertNotIn('{{',(self.root/c['source']['path']).read_text())
            self.assertEqual(set(c['schedules']),{'original','unseen','adversarial'})

    def test_blanket_approval_cannot_replace_independent_review(self):
        cpath=self.root/'private/catalog.json';cpath.write_text('{}');apath=self.root/'private/admission.json';apath.write_text(json.dumps({'schema':SCHEMA,'status':'APPROVED','catalog_sha256':digest(cpath),'cases':{}}))
        with patch('primary_contract.verify_generated',return_value={'cases':[]}):
            with self.assertRaisesRegex(ValueError,'independent human'):verify_admission(self.root,cpath,apath,{})

    def test_primary_without_freeze_cannot_launch_a_model_process(self):
        from run_trial import run_subject
        from types import SimpleNamespace
        with patch('run_trial.verify_freeze',side_effect=ValueError('primary cohort requires a pre-holdout freeze')),patch('run_trial.CodexAppServer',side_effect=AssertionError('MUST NOT LAUNCH')):
            with self.assertRaisesRegex(ValueError,'pre-holdout freeze'):run_subject(SimpleNamespace(cohort='primary'))

    def test_pilot_label_cannot_bypass_post_generation_review_gate(self):
        import run_trial
        from types import SimpleNamespace
        (self.root/'private/primary_generation_claim.json').write_text('OFFLINE_TEST_ONLY')
        with patch.object(run_trial,'ROOT',self.root),patch.object(run_trial,'CodexAppServer',side_effect=AssertionError('MUST NOT LAUNCH')):
            with self.assertRaisesRegex(ValueError,'pilot inference is closed'):
                run_trial.run_subject(SimpleNamespace(cohort='pilot'))

    def test_timeout_and_invalid_pin_logs_are_not_concrete_faults(self):
        for kind in ['incomplete_simulation','incomplete_transactions','insufficient_traffic','missing_or_duplicate_monitor_samples','noninteger_or_missing_pin']:
            self.assertFalse(concrete_symptom({'issues':[{'kind':kind,'cycle':9}]}))
        self.assertTrue(concrete_symptom({'issues':[{'kind':'read_data_mismatch','cycle':9}]}))
        self.assertFalse(concrete_symptom({'issues':[{'kind':'read_data_mismatch'}]}))

    def test_subject_cell_cannot_be_replaced_with_new_output_directory(self):
        catalog=self.root/'private/catalog.json';catalog.write_text('{}')
        auth={'catalog_path':str(catalog),'freeze_sha256':'OFFLINE_ONLY'}
        reserve_subject(self.root,auth,'case_0123456789abcdef_D_1')
        with self.assertRaises(FileExistsError):reserve_subject(self.root,auth,'case_0123456789abcdef_D_1')

    def test_frozen_private_inputs_are_rechecked_on_verify(self):
        import freeze
        frozen={'files':{},'artifacts':{},'protected_inputs':self.protocol['protected_inputs']}
        path=self.root/'config/freeze_receipt.json';path.write_text(json.dumps(frozen))
        with patch.object(freeze,'ROOT',self.root),patch.object(freeze,'FREEZE',path),patch.object(freeze,'inventory',return_value={}),patch.object(freeze,'artifacts',return_value={}):
            self.assertEqual(freeze.verify(required=True),frozen)
            (self.root/'private/design/clean.txt').write_text('CHANGED AFTER FREEZE')
            with self.assertRaisesRegex(ValueError,'hash mismatch'):freeze.verify(required=True)

if __name__=='__main__':unittest.main()
