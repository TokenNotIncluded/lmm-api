#!/usr/bin/env python3
"""Fresh isolated Core + real Pi provider/SDK + fresh Chromium OAuth/stream/budget journey.

No existing database, account, browser profile, production origin, or model supplier is accepted.
The login form is a labelled synthetic fixture; authorization and consent are Core product HTML.
"""
from __future__ import annotations
import argparse
import hashlib
import fcntl
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import subprocess
import sys
import time
import urllib.parse
import urllib.request

sys.dont_write_bytecode=True
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tests/architecture_completion'))
from pg_fixture import Fixture, FixtureError, write_private
from build_lease import private_directory

PIN='f0c54764eb3d73e50790346f0dbab834587fe27b'

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def checked(path, expected=None):
    path=Path(path)
    if not path.is_absolute() or path.is_symlink() or not path.is_file() or path.resolve()!=path:
        raise FixtureError('explicit regular canonical local file required')
    if expected is not None and (not re.fullmatch('[a-f0-9]{64}',expected) or digest(path)!=expected):
        raise FixtureError('frozen input digest changed')
    return path

def ports():
    values=set()
    for _ in range(100):
        with socket.socket() as s:
            s.bind(('127.0.0.1',0));port=s.getsockname()[1]
            if port>=49152: values.add(port)
        if len(values)==3: return list(sorted(values))
    raise FixtureError('could not allocate three high numeric loopback ports')

def run(command, env, *, data=None, timeout=60):
    result=subprocess.run(command,input=data,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,timeout=timeout)
    if result.returncode: raise FixtureError('local product operation failed; private raw diagnostics retained by caller')
    return result.stdout

def json_get(url):
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url,timeout=2) as response:
        return json.load(response)

def wait_file(path, child, limit=60):
    until=time.monotonic()+limit
    while not path.exists():
        if child.poll() is not None: raise FixtureError('owned Node runner exited before expected stage')
        if time.monotonic()>until: raise FixtureError('owned process stage timed out')
        time.sleep(.1)

def stop(child):
    if child and child.poll() is None:
        child.send_signal(signal.SIGTERM)
        try: child.wait(timeout=15)
        except subprocess.TimeoutExpired:
            child.kill();child.wait(timeout=5)
            raise FixtureError('owned process required forced termination')

def process_binding(child, binary, expected):
    if child.poll() is not None: raise FixtureError('owned Core exited before final observation')
    executed=Path(f'/proc/{child.pid}/exe')
    raw=Path(f'/proc/{child.pid}/stat').read_text()
    ticks=raw[raw.rfind(')')+1:].split()[19]
    current=binary.stat();actual=executed.stat()
    if digest(executed)!=expected or (current.st_dev,current.st_ino)!=(actual.st_dev,actual.st_ino):
        raise FixtureError('executed Core identity differs from sealed artifact')
    return {'pid':child.pid,'start_ticks':ticks,'device':actual.st_dev,'inode':actual.st_ino,'sha256':expected}

def source_binding(provider):
    env={'PATH':'/usr/bin:/bin','GIT_CONFIG_NOSYSTEM':'1','GIT_CONFIG_GLOBAL':'/dev/null','GIT_OPTIONAL_LOCKS':'0'}
    head=run(['git','-C',str(provider),'rev-parse','HEAD'],env).decode().strip()
    clean=run(['git','-C',str(provider),'status','--porcelain','--untracked-files=all'],env).strip()
    if head!=PIN or clean: raise FixtureError('reviewed Pi provider commit and clean source required')
    paths=run(['git','-C',str(provider),'ls-files','-z'],env).decode().split('\0')
    return {name:digest(provider/name) for name in paths if name}

def url_for(selected,purpose,fixture):
    role=selected['installer'] if purpose=='owner' else selected['runtime']
    secret=(fixture.root/'A10'/f'{purpose}.pgpass').read_text().strip().split(':')[-1]
    url=f"postgresql://{urllib.parse.quote(role,safe='')}:{secret}@127.0.0.1:{selected['port']}/{selected['database']}"
    if purpose=='owner': url+='?options=-c%20role%3D'+selected['owner']
    return url

def seed(fixture,database,username):
    # Owner-only artificial funds. Real registration supplies native provenance;
    # the seed is balanced, append-only and sealed by the actual ledger triggers.
    user=json.loads(fixture.sql("SELECT json_build_object('user_id',u.id,'account_id',u.personal_account_id) FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE p.username='"+username+"'",database))
    uid,account=user['user_id'],user['account_id']
    fixture.sql(f"""BEGIN;
INSERT INTO core_identity.developer_access(user_id,granted,source_reference) VALUES ({uid},TRUE,'synthetic:pi-core-process');
INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES ({uid},'pi-fixture',TRUE,'synthetic:pi-core-process');
INSERT INTO core_billing.ledger_accounts(owner_account_id,bucket) SELECT {account},v FROM unnest(ARRAY['wallet','reserved','debt','affiliate']) v;
WITH j AS (INSERT INTO core_billing.ledger_journals(scope,operation_key,kind,payer_id,payee_id,amount_units,payment_reference,actor_user_id,reason)
 SELECT 'pi-core-fixture','opening','credit',c.id,w.id,1000,'synthetic:pi-core-process',{uid},'Owner-only synthetic fixture funds, not payment acceptance'
 FROM core_billing.ledger_accounts c JOIN core_billing.ledger_accounts w ON w.owner_account_id={account} AND w.bucket='wallet' WHERE c.owner_account_id IS NULL AND c.bucket='clearing' RETURNING *)
INSERT INTO core_billing.ledger_entries(journal_id,ledger_account_id,delta_units,balance_after_units,balance_revision)
 SELECT j.id,v.account,v.delta,0,1 FROM j CROSS JOIN LATERAL (VALUES(j.payer_id,-1000),(j.payee_id,1000)) v(account,delta);
INSERT INTO core_billing.ledger_operations(scope,operation_key,request,result) VALUES ('pi-core-fixture','opening','{{"synthetic_fixture":true}}', '{{"status":"posted","synthetic_fixture":true}}');
COMMIT;""",database)
    return user

def snapshot(fixture,database):
    return json.loads(fixture.sql("""SELECT json_build_object(
'families',(SELECT json_agg(json_build_object('client_id',client_id,'purpose',purpose,'budget_credits',budget_credits,'revoked',revoked_at IS NOT NULL,'scope_count',cardinality(scopes))) FROM core_oauth.families),
'refresh_total',(SELECT count(*) FROM core_oauth.refresh_tokens),'refresh_consumed',(SELECT count(*) FROM core_oauth.refresh_tokens WHERE consumed_at IS NOT NULL),
'charges',(SELECT json_agg(json_build_object('state',state,'reserved',reserved,'settled',settled,'refunded',refunded,'key_null',key_id IS NULL,'oauth_client',oauth_source->>'client_id','budget',oauth_source->>'budget_credits')) FROM core_charging.requests),
'wallet',(SELECT balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='wallet'),
'reserved',(SELECT balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='reserved'),
'unbalanced',(SELECT count(*) FROM (SELECT j.id FROM core_billing.ledger_journals j JOIN core_billing.ledger_entries e ON e.journal_id=j.id GROUP BY j.id HAVING sum(e.delta_units)<>0 OR count(*)<2) q));""",database))

def main():
    if sys.flags.optimize: raise FixtureError('optimized Python disables acceptance assertions')
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--core-binary',type=Path,required=True);parser.add_argument('--core-sha256',required=True)
    parser.add_argument('--node',type=Path,required=True);parser.add_argument('--sdk',type=Path,required=True)
    parser.add_argument('--chromium',type=Path,required=True);parser.add_argument('--playwright',type=Path,required=True)
    parser.add_argument('--output-root',type=Path,required=True);parser.add_argument('--run',action='store_true')
    args=parser.parse_args()
    core=checked(args.core_binary,args.core_sha256);node=checked(args.node);chromium=checked(args.chromium)
    provider=ROOT/'packages/pi-lmm-provider'
    sdk=args.sdk.resolve();pw=args.playwright.resolve()
    if json.loads((sdk/'package.json').read_text())['version']!='1.2.0': raise FixtureError('Pi 1.2.0 SDK required')
    if json.loads((pw/'package.json').read_text())['name']!='playwright-core': raise FixtureError('official Playwright core required')
    output=args.output_root
    allowed=ROOT/'output/remaining-20261020/pi-core'
    if not output.is_absolute() or output.resolve()!=output or not output.is_relative_to(allowed): raise FixtureError('bounded canonical Pi-Core output root required')
    if not output.exists(): output.mkdir(mode=0o700)
    private_directory(output)
    source=source_binding(provider)
    references=[Path(__file__).resolve(),Path(__file__).with_suffix('.mjs').resolve(),ROOT/'tests/architecture_completion/pg_fixture.py',ROOT/'tests/architecture_completion/build_lease.py',ROOT/'deployment/postgres/grant-core-runtime.sql',sdk/'package.json',sdk.parents[2]/'package-lock.json',pw/'package.json',pw.parents[1]/'package-lock.json',node,chromium]
    binding={str(path):digest(checked(path)) for path in references}
    manifest={'core_sha256':args.core_sha256,'provider_commit':PIN,'provider_files':source,'reference_files':binding,'scope':'actual Core process + actual provider/SDK + fresh real Chromium consent; synthetic login harness and upstream'}
    if not args.run:
        print(json.dumps({'status':'contract-ready','run_requested':False,'provider_commit':PIN,'scope':manifest['scope']}));return 0
    lock_path=ROOT/'output/progress-20261020/development/heavy.lock'
    lease=os.open(lock_path,os.O_RDWR|os.O_NOFOLLOW)
    fcntl.flock(lease,fcntl.LOCK_EX)
    work=output/('run-'+secrets.token_hex(8));work.mkdir(mode=0o700);private_directory(work)
    secrets_dir=work/'secrets';secrets_dir.mkdir(mode=0o700);private_directory(secrets_dir)
    write_private(work/'inputs.json',json.dumps(manifest,sort_keys=True)+'\n')
    env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8','TMPDIR':str(work),'XDG_CACHE_HOME':str(work/'browser-cache'),'XDG_CONFIG_HOME':str(work/'browser-config')}
    fixture=Fixture(work);fixture.start();selected=fixture.allocate('A10')
    node_child=None;core_child=None;success=False
    browser_tmp_fd=os.open(work,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:
        database,owner,runtime=(selected[k] for k in ('database','owner','runtime'))
        for purpose in ('owner','runtime'): write_private(secrets_dir/(purpose+'.url'),url_for(selected,purpose,fixture)+'\n')
        owner_config=work/'owner.toml';write_private(owner_config,f'version=1\n[database]\nurl_file={json.dumps(str(secrets_dir/"owner.url"))}\nmax_connections=1\n')
        init=subprocess.run([str(core),'--config',str(owner_config),'init-db'],capture_output=True,env=env,timeout=60)
        write_private(work/'init.stdout',init.stdout.decode(errors='replace'));write_private(work/'init.stderr',init.stderr.decode(errors='replace'))
        if init.returncode: raise FixtureError('fresh Core init-db failed; exact private diagnostics retained')
        grant=(ROOT/'deployment/postgres/grant-core-runtime.sql').read_text().replace("'lmm_core_owner'",repr(owner))
        fixture.sql(f'\\set runtime_role {runtime}\n\\set enable_models true\n\\set enable_web_identity true\n\\set enable_oauth2 true\n'+grant,database)
        core_port,upstream_port,harness_port=ports();issuer=f'http://127.0.0.1:{core_port}'
        supplier_secret='lmm-isolated-test-'+secrets.token_hex(16);password=secrets.token_hex(10);username='pi_fixture_'+secrets.token_hex(4)
        write_private(secrets_dir/'upstream.token',supplier_secret);write_private(secrets_dir/'web.key',secrets.token_hex(32))
        model=f"""version=1
input_token_ceiling=100
lease_seconds=20
timeout_seconds=10
max_output_tokens=20
local_streaming=true
credentials=[{{reference={{id=7,revision=1}},secret_file={json.dumps(str(secrets_dir/'upstream.token'))}}}]
key_groups=[]
[routing]
version=1
groups=[{{name='pi-fixture',enabled=true}}]
upstreams=[{{id=1,endpoint='http://127.0.0.1:{upstream_port}/upstream/v1',protocol='openai_chat',credential={{id=7,revision=1}},enabled=true}}]
routes=[{{id=1,model='gpt-4o-mini',group='pi-fixture',enabled=true,speeds=['standard'],targets=[{{id=1,upstream_id=1,upstream_model='gpt-4o-mini',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}}],failover={{max_attempts=1,retry_on=[],allow_replay_after_send=false}}}}]
aliases=[]
[prices]
version=1
rules=[{{id=1,model='gpt-4o-mini',group='pi-fixture',speed='standard',rates={{input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0}},group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000}}]
"""
        write_private(work/'model.toml',model)
        config=f"""version=1
[server]
listen='127.0.0.1:{core_port}'
worker_threads=2
max_inflight=16
drain_timeout_seconds=3
[database]
url_file={json.dumps(str(secrets_dir/'runtime.url'))}
max_connections=3
[identity]
registration_enabled=true
password_slots=1
[identity.web]
origin='{issuer}'
refresh_key_file={json.dumps(str(secrets_dir/'web.key'))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
[identity.oauth2]
issuer='{issuer}'
pi_max_credits=200
groups=['pi-fixture']
access_ttl_seconds=300
family_ttl_seconds=600
isolated_loopback_test=true
isolated_secret_directory={json.dumps(str(secrets_dir))}
[model]
config_file={json.dumps(str(work/'model.toml'))}
local_http=true
"""
        write_private(work/'core.toml',config)
        values={'output':str(work),'secrets':str(secrets_dir),'issuer':issuer,'upstream_port':upstream_port,'harness_port':harness_port,'sdk':str(sdk),'playwright':str(pw),'chromium':str(chromium),'provider':str(provider),'username':username,'password':password,'supplier_secret':supplier_secret}
        write_private(work/'node-config.json',json.dumps(values)+'\n')
        node_log=(work/'node.log').open('xb');os.chmod(work/'node.log',0o600)
        node_env={**env,'TMPDIR':f'/proc/{os.getpid()}/fd/{browser_tmp_fd}'}
        node_child=subprocess.Popen([str(node),str(Path(__file__).with_suffix('.mjs')),str(work/'node-config.json')],env=node_env,stdout=node_log,stderr=subprocess.STDOUT)
        wait_file(work/'transport-ready.json',node_child)
        checked(core,args.core_sha256)
        core_log=(work/'core.log').open('xb');os.chmod(work/'core.log',0o600)
        core_child=subprocess.Popen([str(core),'--config',str(work/'core.toml')],env=env,stdin=subprocess.DEVNULL,stdout=core_log,stderr=subprocess.STDOUT)
        until=time.monotonic()+20
        while True:
            if core_child.poll() is not None: raise FixtureError('Core process exited; inspect private core.log')
            try:
                ready=json_get(issuer+'/health/ready')
                if ready['capabilities']['models'] is True: break
            except (OSError,ValueError,KeyError): pass
            if time.monotonic()>until: raise FixtureError('Core model readiness timed out')
            time.sleep(.1)
        process_before=process_binding(core_child,core,args.core_sha256)
        write_private(work/'core-process-start.json',json.dumps(process_before)+'\n')
        request=urllib.request.Request(issuer+'/api/user/register',data=json.dumps({'username':username,'password':password,'email':'','accepted_legal':True}).encode(),headers={'Content-Type':'application/json','Origin':issuer},method='POST')
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request,timeout=10) as response: assert json.load(response)['success'] is True
        user=seed(fixture,database,username)
        write_private(work/'begin-login.json',json.dumps({'synthetic_user':True,'seed_complete':True})+'\n')
        status=node_child.wait(timeout=180)
        if status!=0 or not (work/'provider-result.json').exists(): raise FixtureError('actual provider/browser journey failed; private evidence retained')
        provider_result=json.loads((work/'provider-result.json').read_text());state=snapshot(fixture,database)
        assert provider_result['supplier_hits']==2 and provider_result['real_chromium'] is True
        assert state['wallet']==902 and state['reserved']==0 and state['unbalanced']==0
        assert len(state['families'])==1 and state['families'][0]['budget_credits']==200 and state['families'][0]['revoked'] is True
        assert state['refresh_total']==2 and state['refresh_consumed']==1
        assert len(state['charges'])==2 and all(c['state']=='settled' and c['settled']==49 and c['key_null'] is True and c['oauth_client']=='lmm-pi' for c in state['charges'])
        checked(core,args.core_sha256)
        if process_binding(core_child,core,args.core_sha256)!=process_before: raise FixtureError('Core process identity drifted')
        if source_binding(provider)!=source: raise FixtureError('provider source drifted during run')
        if any(digest(Path(path))!=expected for path,expected in binding.items()): raise FixtureError('runner/helper/tool input drifted during run')
        write_private(work/'database-result.json',json.dumps(state,sort_keys=True)+'\n')
        success=True
        summary={'status':'passed','actual_core_process':True,'actual_pi_provider_sdk':True,'actual_chromium_authorization_consent':True,'signin_ui':'synthetic-harness; actual Core login API','production_or_paid_model_contact':False,'budget_rejection_before_supplier':True,'supplier_hits':2,'wallet_opening':1000,'wallet_closing':902,'settled_credits':98,'core_sha256':args.core_sha256,'provider_commit':PIN,'scope':'limited local Pi-Core OAuth/model/budget integration, not full Web/client or production acceptance'}
    except Exception as error:
        write_private(work/'failure.json',json.dumps({'status':'failed','type':type(error).__name__,'message':str(error) if isinstance(error,FixtureError) else 'assertion or library failure; inspect private diagnostics','resource_retained':True})+'\n')
        raise
    finally:
        cleanup_errors=[]
        for child in (node_child,core_child):
            try: stop(child)
            except Exception as error: cleanup_errors.append(type(error).__name__)
        try:
            if success and not cleanup_errors: fixture.cleanup()
        except Exception as error: cleanup_errors.append(type(error).__name__)
        finally: os.close(browser_tmp_fd);fcntl.flock(lease,fcntl.LOCK_UN);os.close(lease)
        if cleanup_errors:
            write_private(work/'cleanup-failure.json',json.dumps({'status':'failed','errors':cleanup_errors,'resources_retained':True})+'\n')
            raise FixtureError('owned process/fixture cleanup incomplete; original evidence retained')
    write_private(work/'result.json',json.dumps({**summary,'owned_processes_reaped':True,'owned_container_removed':True},sort_keys=True)+'\n')
    print(json.dumps({'status':'passed','receipt':str(work/'result.json'),'container_cleanup':'confirmed'}));return 0

if __name__=='__main__':
    try: raise SystemExit(main())
    except Exception as error:
        print(json.dumps({'status':'failed','error_type':type(error).__name__,'detail':'private scoped evidence retained; no automatic replay'}));raise SystemExit(3)
