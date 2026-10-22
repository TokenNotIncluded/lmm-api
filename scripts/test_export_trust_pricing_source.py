"""Producer serialization/coverage regressions, not original-Go or PG signoff."""
from copy import deepcopy
from decimal import Decimal
import importlib.util
import json
import hashlib
from pathlib import Path
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('producer',Path(__file__).with_name('export-trust-pricing-source.py'))
P=importlib.util.module_from_spec(spec);spec.loader.exec_module(P)


def fixture():
 binding=dict(source_dump_sha256='a'*64,source_cutoff=200,source_commit=P.REV)
 records=[{'user_id':2,'platform_role':'user'}]
 identities={'records':records,'mapping_sha256':P.digest({'source_sha256':'a'*64,'records':records})}
 user=dict(source_user_id=2,projection_known=True,orders=[dict(source_order_id=7,net_credits=0,paid_at=100)])
 raw=dict(id=7,user_id=2,settled_amount_micros=0,money=Decimal('1.25'),settlement_currency='',create_time=80,complete_time=100)
 order=dict(source_order_id=7,source_user_id=2,credited_credits=500000,refunded_credits=500000,net_credits=0,paid_at=100,projection_known=True,rebase_cutoff=0,projection_mode='fully_refunded')
 # This fixed returned original record is only a serialization fixture, never
 # an oracle for what the legacy evaluator should return for business inputs.
 quote=dict(source_user_id=2,source_role=1,status='available',returned_discount_ratio='0.97',raw_trust_level_info={'level':2,'level_source':'automatic','discount_ratio':Decimal('.97')})
 capture=dict(users=[user],raw_tables={'users':[{'id':2}],'top_ups':[raw],'wallet_credit_rebases':[],'wallet_topup_credit_rebases':[]},captured_quotes=[quote],captured_orders=[order],unknown_order_ids=[],projector_invalid=False,projector_error='',transaction_read_only='on',transaction_isolation='repeatable read')
 return capture,binding,identities


class ProducerTests(unittest.TestCase):
 def package(self,capture=None):
  c,b,i=fixture();return P.package_capture(capture or c,b,i,'b'*64,{'old.go':'c'*64})
 def test_exact_returned_ratio_zero_net_and_blank_currency(self):
  basis,evidence,report=self.package()
  self.assertEqual(evidence['users'][0]['original_quote']['discount'],{'numerator':97,'denominator':100})
  self.assertEqual(evidence['orders'][0]['original_currency'],'')
  self.assertEqual(evidence['orders'][0]['original_amount'],'1.25')
  self.assertEqual(basis['users'][0]['orders'][0]['net_credits'],0)
  self.assertEqual(evidence['basis_sha256'],P.digest(basis))
  self.assertEqual(evidence['independent_report_sha256'],P.digest(report))
  self.assertEqual(evidence['orders'][0]['net_evidence_sha256'],P.digest(report['orders'][0]))
 def test_opaque_currency_unchanged(self):
  c,_,_=fixture();c['raw_tables']['top_ups'][0]['settlement_currency']='XYZ-old'
  self.assertEqual(self.package(c)[1]['original_currencies'],['XYZ-old'])
 def test_unknown_source_not_turned_into_usable_evidence(self):
  c,_,_=fixture();c['unknown_order_ids']=[7]
  with self.assertRaisesRegex(ValueError,'UNAVAILABLE_ORIGINAL'):self.package(c)
 def test_missing_currency_is_not_empty_string(self):
  c,_,_=fixture();c['raw_tables']['top_ups'][0].pop('settlement_currency')
  with self.assertRaisesRegex(ValueError,'CURRENCY'):self.package(c)
 def test_missing_user_coverage(self):
  c,_,_=fixture();c['users']=[]
  with self.assertRaisesRegex(ValueError,'COVERAGE'):self.package(c)
 def test_duplicate_raw_order_refused(self):
  c,_,_=fixture();c['raw_tables']['top_ups']*=2
  with self.assertRaisesRegex(ValueError,'DUPLICATE_RAW'):self.package(c)
 def test_future_source_timestamp_refused(self):
  c,_,_=fixture();c['raw_tables']['top_ups'][0]['create_time']=201
  with self.assertRaisesRegex(ValueError,'FUTURE_TIMESTAMP'):self.package(c)
 def test_mapping_digest_and_duplicate_refused(self):
  c,b,i=fixture();i['records']*=2
  with self.assertRaisesRegex(ValueError,'COVERAGE'):P.package_capture(c,b,i,'b'*64,{})
  c,b,i=fixture();i['mapping_sha256']='d'*64
  with self.assertRaisesRegex(ValueError,'DIGEST'):P.package_capture(c,b,i,'b'*64,{})
 def test_unicode_mapping_uses_identity_domain_only(self):
  c,b,i=fixture();i['records'][0]['username']='迁移用户🙂'
  material={'source_sha256':b['source_dump_sha256'],'records':i['records']}
  encoded=json.dumps(material,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()
  i['mapping_sha256']=hashlib.sha256(encoded).hexdigest()
  self.assertEqual(P.exact_ids(i,b['source_dump_sha256']),[2])
  self.assertNotEqual(i['mapping_sha256'],P.digest(material))
  P.package_capture(c,b,i,'b'*64,{})
  # A digest from the trust evidence domain must still be rejected by A2.
  i['mapping_sha256']=P.digest(material)
  with self.assertRaisesRegex(ValueError,'IDENTITY_MAPPING_DIGEST_INVALID'):
   P.exact_ids(i,b['source_dump_sha256'])
  self.assertIn('\\u8fc1',P.canonical(material))
 def lifecycle_fixture(self):
  receipt=Path('/tmp/source-receipt.private.json')
  reader={'port':55489,'socket_directory':'/tmp/source-private-socket'}
  life={'pid':12345,'data':'/tmp/source-private-data','socket':reader['socket_directory'],
        'port':reader['port'],'receipt':str(receipt),'read_only':True,
        'reader_privileges_verified':True,'archive_hash_verified':True,'source_business_writes':0}
  return life,reader,receipt
 def test_current_and_legacy_lifecycle_contracts(self):
  life,reader,receipt=self.lifecycle_fixture()
  self.assertEqual(P.validate_lifecycle(life,reader,receipt),12345)
  legacy={k:life[k]for k in ('data','socket','port')};legacy['postmaster_pid']=12345
  self.assertEqual(P.validate_lifecycle(legacy,reader,receipt),12345)
  life['postmaster_pid']=12345
  self.assertEqual(P.validate_lifecycle(life,reader,receipt),12345)
 def test_conflicting_pids_refused(self):
  life,reader,receipt=self.lifecycle_fixture();life['postmaster_pid']=23456
  with self.assertRaisesRegex(ValueError,'PID_CONTRADICTION'):P.validate_lifecycle(life,reader,receipt)
 def test_lifecycle_attestations_and_paths_remain_strict(self):
  for key,value,code in [('read_only',False,'ATTESTATION'),('reader_privileges_verified',None,'ATTESTATION'),
       ('archive_hash_verified',False,'ATTESTATION'),('source_business_writes',1,'WRITES'),
       ('receipt','/tmp/other-receipt','RECEIPT'),('data','relative','PATH'),('socket','/tmp/other-socket','LISTENER'),
       ('port',55490,'LISTENER'),('pid',True,'PID_INVALID')]:
   with self.subTest(key=key):
    life,reader,receipt=self.lifecycle_fixture();life[key]=value
    with self.assertRaisesRegex(ValueError,code):P.validate_lifecycle(life,reader,receipt)
 def test_duplicate_json_key_refused(self):
  with tempfile.TemporaryDirectory()as d:
   p=Path(d)/'input.json';p.write_text('{"x":1,"x":2}')
   with self.assertRaisesRegex(ValueError,'DUPLICATE_JSON'):P.read_json(p)
 def test_unavailable_quote_stays_unavailable(self):
  c,_,_=fixture();c['captured_quotes'][0]={'source_user_id':2,'source_role':1,'status':'unavailable'}
  self.assertEqual(self.package(c)[1]['users'][0]['original_quote'],{'status':'unavailable'})
 def test_adapter_uses_exact_old_authority_cutoff_and_no_startup(self):
  main=Path(__file__).with_name('trust_pricing').joinpath('source_export_main.go.txt').read_text()
  model=Path(__file__).with_name('trust_pricing').joinpath('source_export_model.go.txt').read_text()
  self.assertIn('ReadOnly: true',main);self.assertIn('TRUST_SOURCE_CUTOFF',main)
  self.assertNotIn('1214',main);self.assertIn('getFreshPaidTopUpAggregatesDB(DB',model)
  self.assertIn('evaluateTrustLevelCredits(',model);self.assertIn('cutoff, policy.trustConfiguration',model)
  self.assertNotIn('time.Now()',model);self.assertNotIn('InitDB(',main);self.assertNotIn('AutoMigrate',main)
  self.assertNotIn('effective_quote(',Path(P.__file__).read_text())

if __name__=='__main__':unittest.main()
