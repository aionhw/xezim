import json, unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
class ContractTests(unittest.TestCase):
 def test_native_recipe_matches_published_field_shape(self):
  d=json.loads((ROOT/'protocol/NATIVE_RECIPE_PORT_REQUIRED.json').read_text())
  self.assertEqual(d['schema_version'],1)
  self.assertEqual(d['trigger']['correlation_key'],'txn_id')
  self.assertEqual(set(d['capture']['fields']),{'time','phase','object','old','new','writer','source','txn_id'})
  self.assertLessEqual(d['capture']['max_events'],8192)
  self.assertLessEqual(d['capture']['max_bytes'],16777216)
  self.assertLessEqual(d['capture']['pretrigger_events'],4096)
 def test_model_and_api_contract(self):
  d=json.loads((ROOT/'config/model.json').read_text())
  self.assertEqual(d['model'],'gpt-6.1-sol')
  self.assertEqual(d['reasoning_effort'],'medium')
  self.assertTrue(d['no_openai_api_key_required'])
  self.assertEqual(d['authentication'].split(' (')[0], 'Codex app-server using Sign in with ChatGPT')
  self.assertFalse(d['zero_cached_token_requirement'])
  self.assertEqual(d['primary_cohort']['scheduled_sessions'],168)
 def test_no_published_private_answers(self):
  self.assertFalse((ROOT/'private/answer_keys.json').exists())
  self.assertGreaterEqual(json.loads((ROOT/'results/STATUS.json').read_text())['real_model_subscription_sessions'],0)
if __name__=='__main__':unittest.main()
