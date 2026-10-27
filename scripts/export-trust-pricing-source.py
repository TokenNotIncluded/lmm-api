#!/usr/bin/env python3
"""Read original cc52 NET/audit functions in ONE readonly restored-source tx.
No startup/restore/migration/target DB writes. All material stays private.
Requires the independently verified source receipt and frozen importer helper.
"""
from pathlib import Path
import argparse,base64,fcntl,hashlib,importlib.util,io,json,os,re,subprocess,sys,tarfile
from decimal import Decimal
from fractions import Fraction
REV = 'cc52aa195c6f2a01d703a1a407be73c31b89867d'
PROJECTION = 'legacy-cc52-trustPaidCreditSQL-net-v1'
PROJECTOR_PATHS = ('topup.go','trust_level.go','trust_paid_credit_projection.go',
                   'user_topup_projection.go','payment_refund.go','payment_refund_rebase.go')
def canonical(value):
 return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=True,
                   default=lambda v: format(v,'f') if isinstance(v,Decimal) else (_ for _ in ()).throw(ValueError('JSON_TYPE')))
def digest(value): return hashlib.sha256(canonical(value).encode()).hexdigest()
def require(ok,code):
 if not ok: raise ValueError(code)
def read_json(path):
 def pairs(entries):
  obj={}
  for key,value in entries:
   require(key not in obj,'DUPLICATE_JSON_KEY');obj[key]=value
  return obj
 return json.loads(path.read_text(),parse_float=Decimal,object_pairs_hook=pairs,
                   parse_constant=lambda _: (_ for _ in ()).throw(ValueError('NONFINITE_JSON')))
def exact_ids(mapping,archive_sha):
 records=mapping.get('records');require(type(records)is list and records,'IDENTITY_MAPPING_INVALID')
 ids=[v['user_id']for v in records]
 require(all(type(v)is int and v>0 for v in ids)and ids==sorted(set(ids)),'IDENTITY_COVERAGE_INVALID')
 # Identity A2 has its own hash domain: UTF-8 Unicode, unlike the trust
 # consumer's escaped-ASCII evidence/quote domain. Keep these separate.
 mapping_bytes=json.dumps({'source_sha256':archive_sha,'records':records},sort_keys=True,separators=(',',':'),ensure_ascii=False,allow_nan=False).encode()
 require(mapping.get('mapping_sha256')==hashlib.sha256(mapping_bytes).hexdigest(),'IDENTITY_MAPPING_DIGEST_INVALID')
 return ids

def validate_lifecycle(lifecycle,reader,receipt_path):
 """Support the original postmaster_pid and restored-source pid contracts.

 This normalizes field spelling only. It never manufactures attestation fields
 or changes the lifecycle receipt; the original file hash remains bound.
 """
 require(type(lifecycle)is dict,'SOURCE_LIFECYCLE_INVALID')
 pids=[lifecycle[k]for k in ('pid','postmaster_pid')if k in lifecycle]
 require(pids and all(type(p)is int and p>1 for p in pids),'SOURCE_PID_INVALID')
 require(len(set(pids))==1,'SOURCE_PID_CONTRADICTION')
 for key in ('data','socket'):
  value=lifecycle.get(key)
  require(type(value)is str and '\0'not in value and Path(value).is_absolute(),'SOURCE_LIFECYCLE_PATH_INVALID')
 port=lifecycle.get('port')
 require(type(port)is int and 1<=port<=65535 and port==reader['port']
         and lifecycle['socket']==reader['socket_directory'],'SOURCE_LISTENER_MISMATCH')
 if 'pid' in lifecycle:
  for flag in ('read_only','reader_privileges_verified','archive_hash_verified'):
   require(lifecycle.get(flag)is True,'SOURCE_LIFECYCLE_ATTESTATION_MISSING')
  require(type(lifecycle.get('source_business_writes'))is int and lifecycle['source_business_writes']==0,
          'SOURCE_BUSINESS_WRITES_NOT_ZERO')
  require(type(lifecycle.get('receipt'))is str and Path(lifecycle['receipt']).is_absolute()
          and Path(lifecycle['receipt']).resolve()==receipt_path.resolve(),'SOURCE_LIFECYCLE_RECEIPT_MISMATCH')
 return pids[0]

def package_capture(capture,binding,identities,identity_sha,projector_hashes):
 """Serialize old results; never calculate the effective level/discount policy."""
 ids=exact_ids(identities,binding['source_dump_sha256'])
 require([u['source_user_id']for u in capture['users']]==ids,'SOURCE_IDENTITY_COVERAGE_MISMATCH')
 raw=capture['raw_tables']; require([u['id']for u in raw['users']]==ids,'RAW_USER_COVERAGE_MISMATCH')
 cutoff=binding['source_cutoff']
 for row in raw['top_ups']:
  require(all(type(row.get(k))is int and 0<=row[k]<=cutoff for k in ('create_time','complete_time')),'SOURCE_FUTURE_TIMESTAMP')
 basis={k:v for k,v in capture.items()if k not in ('raw_tables','captured_quotes','captured_orders')}
 basis.update(binding)
 quotes=[]
 require([q['source_user_id']for q in capture['captured_quotes']]==ids,'QUOTE_COVERAGE_MISMATCH')
 for q in capture['captured_quotes']:
  quote={'status':q['status']}
  if q['status']=='available':
   info=q['raw_trust_level_info']; fraction=Fraction(Decimal(q['returned_discount_ratio']))
   require(0<fraction<=1 and fraction.denominator<=10**18,'RETURNED_RATIO_UNREPRESENTABLE')
   quote.update(effective_level=info['level'],level_source=info['level_source'],
                discount={'numerator':fraction.numerator,'denominator':fraction.denominator})
  else: require(q['status']=='unavailable','ORIGINAL_QUOTE_STATUS_INVALID')
  quotes.append({**q,'original_quote':quote,'original_quote_sha256':digest(quote)})
 raw_orders={r['id']:r for r in raw['top_ups']};require(len(raw_orders)==len(raw['top_ups']),'DUPLICATE_RAW_ORDER')
 for parent in raw['wallet_credit_rebases']:
  plan=parent['plan']; plan=json.loads(plan,parse_float=Decimal)if type(plan)is str else plan
  require(type(plan.get('snapshot_at'))is int and 0<plan['snapshot_at']<=cutoff,'SOURCE_FUTURE_AUDIT_TIMESTAMP')
 projected=capture['captured_orders'];order_ids=[r['source_order_id']for r in projected]
 require(order_ids==sorted(set(order_ids)),'DUPLICATE_PROJECTED_ORDER')
 basis_orders=[(u['source_user_id'],o)for u in capture['users']for o in u['orders']]
 basis_order_ids=[o['source_order_id']for _,o in basis_orders]
 require(len(basis_order_ids)==len(set(basis_order_ids)),'DUPLICATE_BASIS_ORDER')
 known_orders={p['source_order_id']:p for p in projected if p['projection_known']}
 require(set(basis_order_ids)==set(known_orders),'INCOMPLETE_ORDER_COVERAGE')
 for uid,o in basis_orders:
  p=known_orders[o['source_order_id']]
  require(p['source_user_id']==uid and all(p[k]==o[k]for k in ('paid_at','net_credits')),'ORDER_PROJECTION_MISMATCH')
 # Unknown source outcomes remain in capture/basis. Current consumer cannot
 # represent their missing per-order NET, so emit no usable evidence at all.
 require(not capture['unknown_order_ids'] and not capture['projector_invalid'] and not capture['projector_error'],
         'UNAVAILABLE_ORIGINAL_PROJECTION_RETAINED_IN_CAPTURE')
 proofs=[];audits=[];currencies=set()
 for p in projected:
  require(p['projection_known'] is True,'UNKNOWN_ORDER_PROJECTION')
  oid=p['source_order_id'];row=raw_orders[oid];currency=row.get('settlement_currency')
  require(type(currency)is str and len(currency)<=16 and '\0'not in currency,'UNREPRESENTABLE_ORIGINAL_CURRENCY')
  currencies.add(currency)
  amount=Decimal(row['settled_amount_micros'])/1000000 if row['settled_amount_micros']>0 else Decimal(row['money'])
  require(amount>0 and amount.is_finite(),'UNREPRESENTABLE_ORIGINAL_AMOUNT')
  proof={k:p[k]for k in ('source_order_id','source_user_id','credited_credits','refunded_credits','net_credits','paid_at')}
  audit={**proof,'projection_mode':p['projection_mode'],'rebase_cutoff':p['rebase_cutoff'],
         'projector_result':{k:p[k]for k in ('credited_credits','refunded_credits','net_credits')},
         'source_rows':[{'table':'top_ups','row':row}]}
  if p['projection_mode'] not in ('unrebased','fully_refunded'):
   # Preserve complete audit plans and their child/raw source closure. Plans
   # are JSONB semantic values or observed text; never invent pre-JSONB bytes.
   parents=raw['wallet_credit_rebases'];children=raw['wallet_topup_credit_rebases']
   needed={c['top_up_id']for c in children}|{oid}
   require(needed<=set(raw_orders),'AUDIT_RAW_SOURCE_CLOSURE_MISSING')
   audit['source_rows']=[{'table':'top_ups','row':raw_orders[n]}for n in sorted(needed)]
   audit['source_rows'] += [{'table':'wallet_credit_rebases','row':v}for v in parents]
   audit['source_rows'] += [{'table':'wallet_topup_credit_rebases','row':v}for v in children]
   audit['parent_plan_hashes']={v['migration_id']:{'declared_plan_sha256':v['plan_sha256'],
     'observed_plan_sha256':hashlib.sha256(v['plan'].encode()).hexdigest()if type(v['plan'])is str else digest(v['plan'])}for v in parents}
  proof.update(source_kind='successful_external_paid_topup',projection=PROJECTION,original_currency=currency,
               original_amount=format(amount,'f'),original_row=row,original_row_sha256=digest(row),net_evidence_sha256=digest(audit))
  proofs.append(proof);audits.append(audit)
 basis_sha=digest(basis)
 shared={**binding,'basis_sha256':basis_sha,'identity_mapping_sha256':identity_sha,
         'complete_user_ids':ids,'users':quotes,'projection':PROJECTION,'format_version':1}
 report={**shared,'scope':'original-net-projector-report-v1','orders':audits,
         'transaction_read_only':capture['transaction_read_only'],'transaction_isolation':capture['transaction_isolation'],
         'projector_source_hashes':projector_hashes,'observation_kind':'automatic_original_source_observation_not_owner_approval'}
 evidence={**shared,'scope':'original-trust-evidence-v1','orders':proofs,'credits_per_usd':500000,
           'original_currencies':sorted(currencies),'independent_report_sha256':digest(report)}
 return basis,evidence,report

def main():
 os.umask(0o077)
 a=argparse.ArgumentParser(description=__doc__)
 a.add_argument('--identity-mapping',required=True,type=Path)
 a.add_argument('--source-receipt',required=True,type=Path)
 a.add_argument('--source-lifecycle',required=True,type=Path)
 a.add_argument('--readonly-helper',required=True,type=Path)
 a.add_argument('--helper-sha256',required=True)
 a.add_argument('--archive-sha256',required=True)
 a.add_argument('--source-cutoff',required=True,type=int)
 a.add_argument('--output',required=True,type=Path)
 a.add_argument('--heavy-lock',required=True,type=Path)
 x=a.parse_args();root=Path(__file__).resolve().parents[1]
 def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
 require(sha(x.readonly_helper)==x.helper_sha256,'HELPER_HASH_MISMATCH')
 require(not x.output.exists() and x.output.is_absolute(),'FRESH_PRIVATE_OUTPUT_REQUIRED');x.output.mkdir(mode=0o700)
 r=json.loads(x.source_receipt.read_text());require(r['archive']['sha256']==x.archive_sha256,'ARCHIVE_IDENTITY_MISMATCH')
 exact_ids(read_json(x.identity_mapping),x.archive_sha256)
 require(0<x.source_cutoff<253402300800,'INVALID_CUTOFF')
 lifecycle=read_json(x.source_lifecycle);pid=validate_lifecycle(lifecycle,r['readonly_reader'],x.source_receipt);proc=Path(f'/proc/{pid}')
 require(type(pid)is int and pid>1 and proc.is_dir(),'SOURCE_PROCESS_NOT_ALIVE')
 require(str(lifecycle['data']).encode()in(proc/'cmdline').read_bytes(),'SOURCE_DATA_PROCESS_MISMATCH')
 require(int((Path(lifecycle['data'])/'postmaster.pid').read_text().splitlines()[0])==pid,'SOURCE_PID_MISMATCH')
 start_ticks=(proc/'stat').read_text().rsplit(')',1)[1].split()[19]
 require(lifecycle['port']==r['readonly_reader']['port'] and str(lifecycle['socket'])==r['readonly_reader']['socket_directory'],'SOURCE_LISTENER_MISMATCH')
 spec=importlib.util.spec_from_file_location('reviewed_readonly',x.readonly_helper);I=importlib.util.module_from_spec(spec);sys.modules[spec.name]=I;spec.loader.exec_module(I)
 ident=json.loads(I.psql(r['readonly_reader'],'BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL row_security=off;\n'+I.IDENTITY_SQL+'\nROLLBACK;',readonly=True,private_dir=x.output,label='source-preflight'))
 I.verify_instance(ident,r['readonly_reader'],r['source_instance_identity'],source=True)
 rev=REV
 raw=subprocess.check_output(['git','archive',rev,'apps/api-go'],cwd=root)
 with tarfile.open(fileobj=io.BytesIO(raw))as t:
  require(all(not p.issym() and not p.islnk() and (p.name.startswith('apps/api-go/')or p.name in ['apps','apps/api-go'])for p in t.getmembers()),'SOURCE_ARCHIVE_INVALID')
  t.extractall(x.output/'source',filter='data')
 c=x.output/'source/apps/api-go';original={str(p.relative_to(c)):sha(p)for p in c.rglob('*')if p.is_file()}
 (x.output/'original-source-manifest.json').write_text(json.dumps(original,indent=2)+'\n')
 (c/'model/trust_source_export.go').write_bytes((root/'scripts/trust_pricing/source_export_model.go.txt').read_bytes())
 d=c/'cmd/trust-source-export';d.mkdir(parents=True);(d/'main.go').write_bytes((root/'scripts/trust_pricing/source_export_main.go.txt').read_bytes())
 adapter_paths=(Path(__file__),root/'scripts/trust_pricing/source_export_main.go.txt',root/'scripts/trust_pricing/source_export_model.go.txt')
 adapters={str(p.relative_to(root)):sha(p)for p in adapter_paths}
 require(sha(c/'model/trust_source_export.go')==adapters['scripts/trust_pricing/source_export_model.go.txt'] and sha(d/'main.go')==adapters['scripts/trust_pricing/source_export_main.go.txt'],'ADAPTER_COPY_DRIFT')
 env={k:v for k,v in os.environ.items()if not k.startswith('PG')};env.update({'GOPROXY':'off','GOTOOLCHAIN':'local','GOMAXPROCS':'1'})
 with x.heavy_lock.open('a')as lock:
  fcntl.flock(lock,fcntl.LOCK_EX)
  with (x.output/'compile.private.log').open('w')as log:
   build=subprocess.run(['go','build','-buildvcs=false','-p=1','-mod=readonly','-o',str(x.output/'source-export'),'./cmd/trust-source-export'],cwd=c,env=env,stdout=log,stderr=subprocess.STDOUT)
  require(build.returncode==0,'SOURCE_ADAPTER_BUILD_FAILED')
 require(all(sha(c/p)==d for p,d in original.items()),'ORIGINAL_SOURCE_DRIFT')
 require(re.fullmatch('[a-z_][a-z_0-9]*',r['source']['schema'])is not None,'SOURCE_NAMESPACE_INVALID')
 env.update(I.socket_connection(r['readonly_reader']));env['PGOPTIONS']=I.psql_options(r['readonly_reader'],readonly=True)+' -c search_path='+r['source']['schema']+',pg_catalog';env['TRUST_EXPORT_PATH']=str(x.output/'capture.private.json')
 env['TRUST_SOURCE_CUTOFF']=str(x.source_cutoff);env['TRUST_IDENTITY_SQL_B64']=base64.b64encode(I.IDENTITY_SQL.encode()).decode()
 with (x.output/'runtime.private.log').open('w')as log:
  run=subprocess.run([str(x.output/'source-export')],env=env,stdout=log,stderr=subprocess.STDOUT,timeout=180)
 require(run.returncode==0,'SOURCE_ADAPTER_EXECUTION_FAILED')
 capture=read_json(x.output/'capture.private.json');require(capture['transaction_read_only']=='on' and capture['transaction_isolation']=='repeatable read','READONLY_MODE_MISMATCH')
 I.verify_instance(capture['capture_instance_identity'],r['readonly_reader'],r['source_instance_identity'],source=True)
 require(capture['source_cutoff']==x.source_cutoff,'CUTOFF_EVALUATION_MISMATCH')
 originals={"apps/api-go/model/"+p:original['model/'+p]for p in PROJECTOR_PATHS}
 require(all(sha(p)==adapters[str(p.relative_to(root))]for p in adapter_paths),'ADAPTER_SOURCE_DRIFT')
 binding={'source_commit':rev,'source_dump_sha256':x.archive_sha256,'source_cutoff':x.source_cutoff,
          'source_receipt_sha256':sha(x.source_receipt),'original_manifest_sha256':sha(x.output/'original-source-manifest.json'),
          'capture_sha256':sha(x.output/'capture.private.json'),'binary_sha256':sha(x.output/'source-export'),
          'adapter_source_hashes':adapters,'source_lifecycle_sha256':sha(x.source_lifecycle)}
 (x.output/'capture-provenance.private.json').write_text(canonical(binding))
 basis,evidence,report=package_capture(capture,binding,read_json(x.identity_mapping),sha(x.identity_mapping),originals)
 for name,value in (('basis',basis),('evidence',evidence),('source-review-report',report)):
  (x.output/(name+'.private.json')).write_text(canonical(value))
 require(all(sha(c/p)==d for p,d in original.items()),'ORIGINAL_SOURCE_DRIFT')
 require((proc/'stat').read_text().rsplit(')',1)[1].split()[19]==start_ticks,'SOURCE_PROCESS_CHANGED')
 (x.output/'original-source-manifest.json').write_text(json.dumps(original,indent=2)+'\n')
 (x.output/'receipt.json').write_text(json.dumps({'basis_sha256':sha(x.output/'basis.private.json'),'evidence_sha256':sha(x.output/'evidence.private.json'),'source_review_report_sha256':sha(x.output/'source-review-report.private.json'),'owner_approval_generated':False,'binary_sha256':sha(x.output/'source-export'),'original_source_drift':[],'read_only':True,'users':len(basis['users']),'projection_unknown':sum(not u['projection_known']for u in basis['users'])},indent=2)+'\n')
 print(json.dumps({'users':len(basis['users']),'private_basis_sha256':sha(x.output/'basis.private.json'),'database_writes':False}))

if __name__=="__main__":
 main()
