#!/usr/bin/env python3
"""Prepare owner-only trust-basis SQL; never connect or apply, never change money.
Run only after explicit reviewed 24->25 upgrade and complete identity migration.
"""
from pathlib import Path
from decimal import Decimal
from fractions import Fraction
import argparse,hashlib,json,os,re
os.umask(0o077)
a=argparse.ArgumentParser(description=__doc__)
a.add_argument('--basis',required=True,type=Path);a.add_argument('--basis-sha256',required=True)
a.add_argument('--user-map',required=True,type=Path,help='JSON object of source ID to imported Core ID, exact complete map')
a.add_argument('--user-map-sha256',required=True);a.add_argument('--expected-policy-revision',required=True,type=int)
a.add_argument('--output',required=True,type=Path)
x=a.parse_args()
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def req(b,msg):
 if not b:raise SystemExit(msg)
def integer(v,lo=0,hi=9223372036854775807):
 req(type(v)is int and lo<=v<=hi,'INVALID_INTEGER');return v
def digest(v):req(type(v)is str and re.fullmatch('[a-f0-9]{64}',v)is not None,'INVALID_DIGEST');return v
def sqlstr(v):return "'"+str(v).replace("'","''")+"'"
def ratio(v):
 req(isinstance(v,(Decimal,int))and type(v)is not bool,'INVALID_RATIO');r=Fraction(v)
 req(0<r<=1 and r.denominator<=10**18 and r.numerator<2**64,'UNREPRESENTABLE_RATIO')
 return {'numerator':r.numerator,'denominator':r.denominator}
req(sha(x.basis)==digest(x.basis_sha256)and sha(x.user_map)==digest(x.user_map_sha256),'INPUT_HASH_MISMATCH')
req(not x.output.exists() and x.output.is_absolute(),'FRESH_PRIVATE_OUTPUT_REQUIRED');integer(x.expected_policy_revision,1)
b=json.loads(x.basis.read_text(),parse_float=Decimal);mapping=json.loads(x.user_map.read_text())
req(b['source_commit']=='cc52aa195c6f2a01d703a1a407be73c31b89867d','WRONG_LEGACY_SOURCE')
digest(b['source_dump_sha256']);cutoff=integer(b['source_cutoff'],1,253402300799)
raw=b['legacy_policy'];req(raw['version']==1 and type(raw['paid_activation_enabled'])is bool,'INVALID_POLICY')
policy={'version':1,'paid_activation_enabled':raw['paid_activation_enabled'],'decay_period_days':integer(raw['decay_period_days'],0,3650),'tiers':[]}
req(len(raw['tiers'])==5 and len(raw['role_tiers'])==2,'INVALID_POLICY_TIERS');previous=-1
for i,t in enumerate(raw['tiers']):
 req(integer(t['level'])==i,'INVALID_TIER_LEVEL');n=integer(t['min_paid_credits'],0,2147483647)
 req(n>previous and(i!=0 or n==0),'INVALID_TIER_THRESHOLD');previous=n
 policy['tiers'].append({'level':i,'min_paid_credits':n,'discount':ratio(t['discount_ratio'])})
for i,(level,role,key)in enumerate([(5,10,'admin_discount'),(6,100,'superadmin_discount')]):
 t=raw['role_tiers'][i];req(t['level']==level and t['role']==role,'INVALID_ROLE_POLICY');policy[key]=ratio(t['discount_ratio'])
users=b['users'];req(type(users)is list and len(users)>0,'MISSING_USERS')
ids={str(integer(u['source_user_id'],1))for u in users};req(len(ids)==len(users)and set(mapping)==ids,'INCOMPLETE_OR_DUPLICATE_USER_MAP')
targets=[integer(v,1)for v in mapping.values()];req(len(set(targets))==len(targets),'DUPLICATE_TARGET_USER')
user_rows=[];order_rows=[];seen_orders=set()
for u in users:
 target=mapping[str(u['source_user_id'])];times=[integer(u[k],0,cutoff)for k in ['original_created_at','console_activated_at','last_api_activity_at']]
 override=u['override_level'];req(override is None or type(override)is int,'INVALID_OVERRIDE')
 # The actual old getter maps a corrupt ordinary override to explicit L0.
 if override is not None and not 0<=override<=4:override=0
 req(type(u['projection_known'])is bool,'MISSING_PROJECTION_KNOWN');orders=u['orders'];req(type(orders)is list and len(orders)<=10000,'INVALID_ORDER_SET')
 ud=hashlib.sha256(json.dumps(u,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 user_rows.append((target,*times,override,u['projection_known'],len(orders),1,ud))
 total=0
 for order in orders:
  oid=integer(order['source_order_id'],1);req(oid not in seen_orders,'DUPLICATE_SOURCE_ORDER');seen_orders.add(oid)
  net=integer(order['net_credits']);total+=net;integer(total)
  paid=integer(order['paid_at'],1,cutoff);od=hashlib.sha256(json.dumps(order,sort_keys=True,separators=(',',':')).encode()).hexdigest()
  order_rows.append((oid,target,net,paid,1,od))
root=Path(__file__).resolve().parents[1];ddlsha=hashlib.sha256((root/'services/core/schema/trust_pricing.sql').read_bytes()+b'\n'+(root/'services/core/schema/image_trust_extension.sql').read_bytes()).hexdigest()
policy_json=json.dumps(policy,sort_keys=True,separators=(',',':'))
lines=['-- Trust pricing only. No balances, ledger, account activation or existing business writes.','BEGIN;','SET TRANSACTION ISOLATION LEVEL SERIALIZABLE;',"SET LOCAL lock_timeout='5s';",'SELECT pg_advisory_xact_lock(741803219);',f'''DO $guard$ BEGIN
 IF (SELECT count(*) FROM core_meta.component_contract)<>25 OR NOT EXISTS(SELECT 1 FROM core_meta.component_contract WHERE name='trust-pricing' AND fingerprint=decode('{ddlsha}','hex')) THEN RAISE EXCEPTION 'trust component25 upgrade required'; END IF;
 IF NOT EXISTS(SELECT 1 FROM pg_class WHERE oid='core_trust_pricing.user_bases'::regclass AND relowner=(SELECT oid FROM pg_roles WHERE rolname=current_user)) THEN RAISE EXCEPTION 'installation owner required'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core_trust_pricing.policies WHERE singleton AND (revision={x.expected_policy_revision} OR source_digest=decode('{x.basis_sha256}','hex'))) THEN RAISE EXCEPTION 'trust policy revision conflict'; END IF;
 IF (SELECT count(*) FROM core_identity.users WHERE id IN ({','.join(map(str,sorted(targets)))}))<>{len(targets)} THEN RAISE EXCEPTION 'complete imported identity mapping required'; END IF;
 IF (SELECT native_user_floor FROM core_trust_pricing.policies WHERE singleton)<={max(targets)} THEN RAISE EXCEPTION 'upgrade must follow complete legacy identity import'; END IF;
END $guard$;''']
for r in user_rows:
 vals=[str(v)for v in r[:4]]+['NULL'if r[4]is None else str(r[4]),'TRUE'if r[5]else'FALSE',str(r[6]),str(r[7]),f"decode('{r[8]}','hex')"]
 columns='user_id,original_created_at,console_activated_at,last_api_activity_at,override_level,projection_known,order_count,revision,source_digest'
 lines.append(f"INSERT INTO core_trust_pricing.user_bases({columns}) VALUES({','.join(vals)}) ON CONFLICT(user_id) DO NOTHING;")
 lines.append(f"DO $check$ BEGIN IF NOT EXISTS(SELECT 1 FROM core_trust_pricing.user_bases WHERE ROW({columns}) IS NOT DISTINCT FROM ROW({','.join(vals)})) THEN RAISE EXCEPTION 'trust user basis conflict'; END IF; END $check$;")
for r in order_rows:
 vals=[str(v)for v in r[:5]]+[f"decode('{r[5]}','hex')"];columns='source_order_id,user_id,net_credits,paid_at,revision,source_digest'
 lines.append(f"INSERT INTO core_trust_pricing.legacy_paid_orders({columns}) VALUES({','.join(vals)}) ON CONFLICT(source_order_id) DO NOTHING;")
 lines.append(f"DO $check$ BEGIN IF NOT EXISTS(SELECT 1 FROM core_trust_pricing.legacy_paid_orders WHERE ROW({columns}) IS NOT DISTINCT FROM ROW({','.join(vals)})) THEN RAISE EXCEPTION 'trust order basis conflict'; END IF; END $check$;")
lines.append(f"DO $check$ BEGIN IF EXISTS(SELECT 1 FROM core_trust_pricing.user_bases b WHERE b.user_id IN ({','.join(map(str,sorted(targets)))}) AND b.order_count<>(SELECT count(*) FROM core_trust_pricing.legacy_paid_orders o WHERE o.user_id=b.user_id)) THEN RAISE EXCEPTION 'trust complete order set mismatch'; END IF; END $check$;")
lines.append(f"UPDATE core_trust_pricing.policies SET revision=CASE WHEN source_digest=decode('{x.basis_sha256}','hex') THEN revision ELSE revision+1 END,spec={sqlstr(policy_json)}::jsonb,source_digest=decode('{x.basis_sha256}','hex'),source_cutoff={cutoff} WHERE singleton;")
lines+=['COMMIT;']
x.output.mkdir(mode=0o700);(x.output/'owner-import.private.sql').write_text('\n'.join(lines)+'\n')
receipt={'basis_sha256':x.basis_sha256,'mapping_sha256':x.user_map_sha256,'ddl_sha256':ddlsha,'sql_sha256':sha(x.output/'owner-import.private.sql'),'users':len(users),'verified_net_order_rows':len(order_rows),'projection_unknown':sum(not u['projection_known']for u in users),'applied':False,'money_writes':False,'replay':'exact unchanged policy/bases/order sets accepted; conflicts rollback; corrections require a separate reviewed owner revision'}
(x.output/'plan.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
