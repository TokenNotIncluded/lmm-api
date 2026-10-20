#!/usr/bin/env python3
"""Fresh PostgreSQL + real Core/Go assistant acceptance harness.

Scripted loopback provider/MCP responses are external counting fixtures, never
substitutes for assistant orchestration, Session authority or ledger storage.
Missing live capabilities produce BLOCKED (exit 2), never PASS.
"""
from __future__ import annotations

import argparse
import hashlib
import http.server
import importlib
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import stat
import subprocess
import sys
import threading
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request

from pg_fixture import Fixture, FixtureError, clean_environment, write_private
from build_lease import private_directory

ROOT = Path(__file__).resolve().parents[2]
FEATURE = 'assistant.agent_turn.execute.v1'
FINAL = 'agent-runner-final-confirmed'
BOUND = 1 << 20


class Blocked(RuntimeError):
    pass


class Failed(RuntimeError):
    pass


def digest(data):
    return hashlib.sha256(data).hexdigest()


def private_read(path):
    path = Path(path)
    info = path.lstat()
    if (path.resolve() != path or not stat.S_ISREG(info.st_mode)
            or info.st_uid != os.getuid() or info.st_mode & 0o077 or info.st_size > BOUND):
        raise Blocked('bounded private input required')
    return path.read_bytes()


def frozen_binary(path, expected):
    path = Path(path)
    if not re.fullmatch('[0-9a-f]{64}', expected) or path.resolve() != path:
        raise Blocked('frozen absolute executable and SHA-256 required')
    info = path.lstat()
    if (not stat.S_ISREG(info.st_mode) or info.st_mode & 0o022
            or not os.access(path, os.X_OK) or digest(path.read_bytes()) != expected):
        raise Blocked('reviewed executable changed')
    return path


def url_for(root, task, purpose, role_option=False):
    fields = private_read(root/task/(purpose+'.pgpass')).decode().strip().split(':')
    host, port, database, role, password = fields
    value = f'postgresql://{urllib.parse.quote(role)}:{urllib.parse.quote(password)}@{host}:{port}/{database}?sslmode=disable&connect_timeout=3'
    if role_option:
        resource = json.loads(private_read(root/task/'resource.json'))
        value += '&options=' + urllib.parse.quote('-c role='+resource['owner'])
    return value


def prepare(args):
    parent = Path(args.private_root)
    private_directory(parent)
    fixture = Fixture(parent)
    fixture.start()
    core, go = fixture.allocate('A07'), fixture.allocate('A08')
    for task, record in [('A07', core), ('A08', go)]:
        for purpose in ['owner', 'runtime']:
            write_private(fixture.root/task/(purpose+'.url'), url_for(fixture.root, task, purpose, purpose == 'owner')+'\n')
    paths = [ROOT/'services/extensions/internal/modules/assistant'/name for name in
             ['schema.sql', 'delegation_schema.sql', 'agent_schema.sql']]
    paths += [ROOT/'services/extensions/internal/modules/assistant_services/schema.sql']
    schema = '\n'.join(p.read_text() for p in paths)
    fixture.sql(f'SET ROLE {go["owner"]};\n'+schema, go['database'])
    grants_path = ROOT/'services/extensions/internal/modules/assistant_services/runtime-grants.sql'
    runtime = go['runtime']
    # Base transcript/journal privileges stay separate from the stricter service
    # receipt/confirmation contract. Never grant all-table writes in this schema.
    base_grants = (
        f'SET ROLE {go["owner"]}; REVOKE ALL ON SCHEMA lmm_assistant FROM PUBLIC; '
        f'GRANT USAGE ON SCHEMA lmm_assistant TO {runtime}; '
        f'GRANT SELECT ON lmm_assistant.contract,lmm_assistant.delegation_contract,lmm_assistant.agent_contract TO {runtime}; '
        f'GRANT SELECT,INSERT,UPDATE,DELETE ON lmm_assistant.records TO {runtime}; '
        f'GRANT SELECT,INSERT,UPDATE ON lmm_assistant.conversations,lmm_assistant.delegation_conversations,lmm_assistant.delegation_turns,lmm_assistant.agent_turns TO {runtime}; '
        f'GRANT SELECT,INSERT,DELETE ON lmm_assistant.history_messages TO {runtime}; '
        f'GRANT SELECT,INSERT ON lmm_assistant.turn_receipts TO {runtime}; '
        f'GRANT SELECT,UPDATE ON lmm_assistant.preset_stats TO {runtime}; '
        f'GRANT USAGE ON SEQUENCE lmm_assistant.conversations_id_seq,lmm_assistant.history_messages_id_seq TO {runtime}; '
    )
    service_grants = grants_path.read_text().replace(':"runtime_role"', '"'+runtime+'"')
    fixture.sql(base_grants+service_grants, go['database'])
    initialized = False
    if args.core_binary:
        binary = frozen_binary(args.core_binary, args.core_sha256 or '')
        config = fixture.root/'core-owner.toml'
        write_private(config, 'version=1\n[database]\nurl_file='+json.dumps(str(fixture.root/'A07/owner.url'))+'\nmax_connections=1\n')
        fixture.record('agent-core-init-intent', {'binary_sha256': args.core_sha256, 'database': core['database'], 'replay': False})
        process = subprocess.run([str(binary), '--config', str(config), 'init-db'],
                                 env=fixture.env, capture_output=True, timeout=120)
        write_private(fixture.root/'core-init.log', process.stdout.decode(errors='replace')+process.stderr.decode(errors='replace'))
        if process.returncode:
            raise Failed('fresh Core init failed; retained private diagnostic')
        grant = (ROOT/'deployment/postgres/grant-core-runtime.sql').read_text().replace("'lmm_core_owner'", "'"+core['owner']+"'")
        fixture.sql('\\set runtime_role '+core['runtime']+'\n'+grant, core['database'])
        initialized = True
    fixture.verify()
    manifest = {'format': 'assistant-agent-fresh-fixture-v1', 'root': str(fixture.root),
                'nonce': fixture.nonce, 'container': fixture.container, 'binding': fixture.binding,
                'identity': fixture.identity, 'boot': fixture.boot, 'core': core, 'go': go,
                'core_initialized': initialized,
                'schema_sha256': {str(p.relative_to(ROOT)): digest(p.read_bytes()) for p in paths},
                'runtime_grants_sha256': digest(grants_path.read_bytes()),
                'runtime_grant_contract': 'explicit-base-journal-plus-services-runtime-grants-v1',
                'limits': ['Only assistant Go schemas installed; toolmarket snapshot, real Session, funding, provider and exact browser grant must be installed by their reviewed owners.',
                           'Fresh preparation is not model, funds, migration or browser acceptance.']}
    target = fixture.root/'assistant-agent-fixture.json'
    write_private(target, json.dumps(manifest, indent=2)+'\n')
    return {'status': 'BLOCKED', 'reason': 'fresh fixture prepared; live Core agent capability/runtime config and exact human grant not yet supplied',
            'fixture_manifest': str(target), 'container': fixture.container,
            'core_database': core['database'], 'go_database': go['database'],
            'system_id': fixture.identity['system_id'], 'core_initialized': initialized,
            'resources_retained': True, 'acceptance_passed': False}


def attach(path):
    data = json.loads(private_read(path))
    if data.get('format') != 'assistant-agent-fresh-fixture-v1':
        raise Blocked('manifest was not created by this runner')
    root = Path(data['root'])
    private_directory(root)
    if Path(path) != root/'assistant-agent-fixture.json':
        raise Blocked('fixture manifest moved')
    # Reconstruct only our own exact resource identity, never discover/adopt by
    # prefix or accept a caller-supplied DSN/container/cluster.
    fixture = object.__new__(Fixture)
    fixture.root, fixture.nonce, fixture.container = root, data['nonce'], data['container']
    fixture.binding, fixture.identity, fixture.boot = data['binding'], data['identity'], data['boot']
    fixture.databases = {data['core']['database'], data['go']['database']}
    fixture.allocations = {'A07': data['core'], 'A08': data['go']}
    fixture.env = clean_environment()
    fixture.env['DOCKER_CONFIG'] = str(root/'docker-config')
    fixture.verify()
    for task, key in [('A07', 'core'), ('A08', 'go')]:
        if json.loads(private_read(root/task/'resource.json')) != data[key]:
            raise Blocked('allocated database identity changed')
    return fixture, data


def capabilities(fixture, config):
    socket_path = Path(config['core_socket'])
    if not socket_path.is_relative_to(fixture.root) or not socket_path.is_socket():
        raise Blocked('real owned Core UDS is required')
    token = private_read(config['core_rpc_token_file']).decode().strip()
    if not 32 <= len(token) <= 256:
        raise Blocked('private Core service credential required')
    generated = fixture.root/('grpc-'+secrets.token_hex(5))
    generated.mkdir(mode=0o700)
    try:
        import grpc
        from grpc_tools import protoc
    except ImportError:
        raise Blocked('installed grpcio/grpcio-tools required') from None
    proto_root = ROOT/'contracts/proto'
    code = protoc.main(['grpc_tools.protoc', '-I'+str(proto_root), '--python_out='+str(generated),
                        '--grpc_python_out='+str(generated), str(proto_root/'lmm/core/v1/control.proto')])
    if code:
        raise Blocked('Core control protobuf generation failed')
    sys.path.insert(0, str(generated))
    pb = importlib.import_module('lmm.core.v1.control_pb2')
    rpc = importlib.import_module('lmm.core.v1.control_pb2_grpc')
    try:
        with grpc.insecure_channel('unix:'+str(socket_path)) as channel:
            result = rpc.CoreControlStub(channel).Capabilities(pb.CapabilitiesRequest(),
                metadata=(('authorization', 'Bearer '+token), ('x-lmm-protocol', '1')), timeout=3)
    except grpc.RpcError:
        raise Blocked('real Core Capabilities RPC unavailable or unauthorized') from None
    if result.protocol_major != 1 or not result.identity_available:
        raise Blocked('Core identity/protocol unavailable')
    if FEATURE not in result.features:
        raise Blocked('live Core capability absent: '+FEATURE)
    return {'protocol_major': result.protocol_major, 'features': sorted(result.features)}


class CountingHTTP:
    def __init__(self, config, port=0):
        self.config, self.lock = config, threading.Lock()
        self.model_calls, self.tool_calls, self.requests = 0, 0, []
        fixture = self
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_GET(self):
                if self.path != '/counts':
                    self.send_error(404); return
                self.reply(fixture.snapshot())
            def reply(self, value, code=200):
                raw = json.dumps(value, separators=(',', ':')).encode()
                self.send_response(code)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(raw)))
                self.end_headers(); self.wfile.write(raw)
            def do_POST(self):
                try:
                    length = int(self.headers.get('Content-Length', '-1'))
                    if length < 0 or length > BOUND: raise Failed('bounded content length required')
                    body = self.rfile.read(length); value = json.loads(body)
                    with fixture.lock:
                        fixture.requests.append({'path': self.path, 'sha256': digest(body)})
                        if self.path == '/v1/chat/completions':
                            self.reply(fixture.model(value)); return
                        if self.path != '/mcp': raise Failed('unexpected fixture endpoint')
                        method = value.get('method')
                        result = {}
                        if method == 'initialize':
                            result = {'protocolVersion': '2025-03-26', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'assistant-agent-counter', 'version': '1'}}
                        elif method == 'tools/list':
                            result = {'tools': [{'name': fixture.config.get('mcp_tool_name', 'count_once'), 'description': 'Counting acceptance tool', 'inputSchema': {'type': 'object', 'properties': {}, 'additionalProperties': False}}]}
                        elif method == 'tools/call':
                            if value.get('params', {}).get('name') != fixture.config.get('mcp_tool_name', 'count_once'): raise Failed('unexpected MCP tool')
                            fixture.tool_calls += 1
                            if fixture.tool_calls != 1: raise Failed('duplicate MCP effect')
                            result = {'content': [{'type': 'text', 'text': 'runner-tool-once'}], 'isError': False}
                        elif method == 'notifications/initialized':
                            self.send_response(202); self.end_headers(); return
                        else: raise Failed('unexpected MCP method')
                        self.reply({'jsonrpc': '2.0', 'id': value.get('id'), 'result': result})
                except (ValueError, KeyError, TypeError, Failed):
                    self.reply({'error': 'counting fixture rejected request'}, 422)
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', port), Handler)
    def model(self, request):
        step = self.model_calls
        self.model_calls += 1
        sequence = [('discover_tools', {'names': ['get_tool_market_services', 'get_tool_market_service', 'call_market_tool']}),
                    ('get_tool_market_services', {}),
                    ('get_tool_market_service', {'service_id': self.config['service_id']}),
                    ('call_market_tool', {key: self.config[key] for key in ['service_id', 'tool_id', 'version_id', 'grant_id']} | {'arguments': self.config.get('arguments', {})})]
        if step > len(sequence): raise Failed('duplicate provider round')
        if step > 0:
            results = [m for m in request.get('messages', []) if m.get('role') == 'tool']
            if len(results) < step:
                raise Failed('provider request lacks persisted prior tool results')
            value = json.loads(results[-1]['content'])
            # Exact domain payloads may be nested in the typed result envelope.
            if isinstance(value, dict) and 'result_json' in value:
                value = value['result_json']
                if isinstance(value,str): value = json.loads(value)
            if not isinstance(value,dict) or value.get('ok') is False or value.get('state') in {'unknown','pending','failed'}:
                raise Failed('provider received an unsuccessful previous tool result')
        message = {'role': 'assistant', 'content': FINAL}
        if step < len(sequence):
            name, args = sequence[step]
            names = {t.get('function', {}).get('name') for t in request.get('tools', [])}
            if name not in names: raise Failed('actual model request lacks required discovered tool schema')
            message = {'role': 'assistant', 'content': None, 'tool_calls': [{'id': 'runner-call-'+str(step), 'type': 'function', 'function': {'name': name, 'arguments': json.dumps(args, separators=(',', ':'))}}]}
        return {'id': 'runner-model-'+str(step), 'object': 'chat.completion', 'model': request.get('model'),
                'choices': [{'index': 0, 'message': message, 'finish_reason': 'tool_calls' if step < len(sequence) else 'stop'}],
                'usage': {'prompt_tokens': 10, 'completion_tokens': 5, 'total_tokens': 15}}
    def snapshot(self):
        with self.lock:
            return {'model_calls': self.model_calls, 'tool_calls': self.tool_calls, 'request_hashes': list(self.requests)}
    def start(self):
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
    def close(self):
        self.server.shutdown(); self.server.server_close()


def local_url(value):
    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme != 'http' or parsed.hostname != '127.0.0.1' or not parsed.port or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in {'', '/'}:
        raise Blocked('explicit loopback HTTP URL required')
    return value.rstrip('/')


def http_json(method, url, token='', body=None):
    headers = {'Content-Type': 'application/json'}
    if token: headers['Authorization'] = 'Bearer '+token
    request = urllib.request.Request(url, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
    try:
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=10) as response:
            return response.status, json.loads(response.read(BOUND))
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read(BOUND))


def database_fingerprint(fixture, database):
    rows = fixture.sql("SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND (n.nspname IN ('core_assistant','core_billing','core_charging','core_tool_market','lmm_assistant')) ORDER BY 1,2;", database)
    out = {}
    for line in rows.splitlines():
        schema, name = line.split('|')
        quoted = '.'.join('"'+x.replace('"','""')+'"' for x in [schema, name])
        raw = fixture.sql('SELECT row_to_json(t)::text FROM '+quoted+' t ORDER BY row_to_json(t)::text COLLATE "C";', database)
        out[schema+'.'+name] = digest(raw.encode())
    if not out: raise Blocked('required real persisted domain tables absent')
    return out


def run(args):
    fixture, manifest = attach(args.fixture_manifest)
    if not args.runtime_config: raise Blocked('private runtime config with real Core UDS/Go binary/exact human grant required')
    config = json.loads(private_read(args.runtime_config))
    # Check the actual running Core capability before invoking any assistant.
    caps = capabilities(fixture, config)
    core_binary = frozen_binary(config['core_binary'], config['core_sha256'])
    core_pid = int(config['core_pid'])
    if Path(f'/proc/{core_pid}/exe').resolve() != core_binary:
        raise Blocked('running Core executable identity differs from frozen binary')
    command = Path(f'/proc/{core_pid}/cmdline').read_bytes().split(b'\0')
    if b'--config' not in command or os.fsencode(config['core_config']) not in command:
        raise Blocked('running Core is not using the reviewed fixture configuration')
    core_config = tomllib.loads(private_read(config['core_config']).decode())
    go_config = tomllib.loads(private_read(config['go_config']).decode())
    expected_core = url_for(fixture.root, 'A07', 'runtime')
    expected_go = url_for(fixture.root, 'A08', 'runtime')
    if private_read(core_config['database']['url_file']).decode().strip() != expected_core:
        raise Blocked('Core config is not bound to the newly allocated runtime database')
    if private_read(go_config['assistant']['database_url_file']).decode().strip() != expected_go or go_config['core']['socket'] != config['core_socket']:
        raise Blocked('Go config is not bound to the new GoPG and Core UDS')
    for key in ['service_id', 'tool_id', 'version_id', 'grant_id']:
        if not re.fullmatch('[A-Za-z0-9_.:-]{1,128}', config[key]): raise Blocked('exact reviewed market IDs required')
    grant = config['grant_id'].replace("'", "''")
    approval = fixture.sql("SELECT row_to_json(a)::text FROM core_tool_market.approvals a WHERE grant_id='"+grant+"' AND NOT revoked;", manifest['core']['database'])
    if not approval: raise Blocked('real existing exact human market approval missing; runner never creates grants')
    original_approval = json.loads(approval)
    if original_approval['intent'].get('max_calls') != 1 or original_approval['evidence'].get('actor_kind') != 'human_session':
        raise Blocked('reviewed single-call human approval required')
    binary = frozen_binary(config['go_binary'], config['go_sha256'])
    base = local_url(config['go_url'])
    if go_config['host']['listen'] != urllib.parse.urlsplit(base).netloc:
        raise Blocked('owned Go process configuration does not bind the exact loopback test listener')
    if not isinstance(config.get('fixture_port'),int) or not 1024 <= config['fixture_port'] <= 65535:
        raise Blocked('fixed unprivileged loopback fixture port required before configuring Core/MCP')
    endpoint = CountingHTTP(config, config.get('fixture_port', 0)); endpoint.start()
    actual_endpoint = f'http://127.0.0.1:{endpoint.server.server_port}'
    if config.get('fixture_url') != actual_endpoint:
        endpoint.close(); raise Blocked('Core model and approved MCP must reference the configured counting endpoint')
    process = None
    logs = fixture.root/('go-run-'+secrets.token_hex(5)+'.log')
    log = open(logs, 'xb'); os.chmod(logs,0o600)
    def start_go():
        return subprocess.Popen([str(binary),'--config',config['go_config']],cwd=ROOT,env=fixture.env,stdout=log,stderr=log,start_new_session=True)
    def stop_go(child):
        if child.poll() is None:
            child.send_signal(signal.SIGTERM)
            try: child.wait(timeout=15)
            except subprocess.TimeoutExpired: raise Blocked('owned Go did not drain; process retained') from None
    try:
        process = start_go()
        token = private_read(config['session_file']).decode().strip()
        deadline = time.monotonic()+15
        while True:
            if process.poll() is not None: raise Blocked('real Go host exited before readiness')
            try:
                code, _ = http_json('GET',base+'/api/assistant/status',token)
                if code == 200: break
            except (OSError,ValueError): pass
            if time.monotonic()>deadline: raise Blocked('real Go host readiness timeout')
            time.sleep(.1)
        turn = 'agent-runner-'+secrets.token_hex(8)
        body = {'conversation_id':0,'client_turn_id':turn,'message':'Use the existing exact single-call approved tool once, then report the result.'}
        code, response = http_json('POST',base+'/api/assistant/chat',token,body)
        conversation = response.get('lmm_assistant_history',{}).get('conversation_id') or response.get('data',{}).get('conversation_id')
        if not conversation: raise Failed('assistant returned no durable original conversation ID')
        recovery = base+f'/api/assistant/conversations/{conversation}/turns/{turn}'
        for _ in range(64):
            if code == 200 and response.get('success'): break
            if code != 202: raise Failed('real assistant turn failed')
            time.sleep(.1); code,response=http_json('POST',recovery+'/resume',token,{})
        else: raise Failed('real assistant did not finish bounded multi-round turn')
        if response['choices'][0]['message']['content'] != FINAL: raise Failed('authoritative Core final response missing')
        counts = endpoint.snapshot()
        if counts['model_calls'] != 5 or counts['tool_calls'] != 1: raise Failed('actual provider/MCP counts differ from exact multi-round path')
        core_db, go_db = manifest['core']['database'], manifest['go']['database']
        model_settlements = fixture.sql("SELECT count(*) FROM core_charging.requests WHERE assistant_grant_id IS NOT NULL AND state='settled';", core_db)
        market_settlements = fixture.sql("SELECT count(*) FROM core_tool_market.calls c JOIN core_tool_market.approvals a ON a.id=c.approval_id WHERE a.grant_id='"+grant+"' AND c.receipt->>'state'='committed';", core_db)
        history_messages = fixture.sql(f'SELECT count(*) FROM lmm_assistant.history_messages WHERE conversation_id={int(conversation)};',go_db)
        turn_receipts = fixture.sql(f'SELECT count(*) FROM lmm_assistant.turn_receipts WHERE conversation_id={int(conversation)};',go_db)
        if (model_settlements,market_settlements,history_messages,turn_receipts) != ('5','1','2','1'):
            raise Failed('actual persisted model/tool/history receipts do not match exact execution counts')
        before = {key: database_fingerprint(fixture,manifest[key]['database']) for key in ['core','go']}
        body['conversation_id']=conversation
        code,repeated=http_json('POST',base+'/api/assistant/chat',token,body)
        if code != 200 or repeated != response: raise Failed('same-turn retry changed final receipt')
        stop_go(process); process=start_go()
        # Recovery is retried only as a read; startup must never run DDL.
        for _ in range(100):
            try:
                code,recovered=http_json('GET',recovery,token)
                if code == 200: break
            except OSError: pass
            time.sleep(.1)
        else: raise Failed('restarted Go cannot read original turn')
        if recovered != response: raise Failed('restarted Go changed final answer/receipt')
        after = {key: database_fingerprint(fixture,manifest[key]['database']) for key in ['core','go']}
        if before != after or endpoint.snapshot() != counts: raise Failed('retry/restart changed original model/tool/ledger/history state')
        fixture.verify()
        return {'status':'PASS','capabilities':caps,'conversation_id':conversation,'client_turn_id':turn,'counts':counts,
                'persisted_fingerprints':after,'fixture_manifest':args.fixture_manifest,
                'limits':['Synthetic external model/MCP; real Core/Go/Session/storage/ledger execution.', 'Existing-site migration, browser UI grant creation and physical resource qualification are separate acceptance.']}
    finally:
        if process is not None: stop_go(process)
        log.close(); endpoint.close()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='mode',required=True)
    p=sub.add_parser('prepare');p.add_argument('--private-root',required=True);p.add_argument('--core-binary');p.add_argument('--core-sha256')
    p=sub.add_parser('check');p.add_argument('--fixture-manifest',required=True)
    p=sub.add_parser('run');p.add_argument('--fixture-manifest',required=True);p.add_argument('--runtime-config')
    p=sub.add_parser('serve');p.add_argument('--script-config',required=True);p.add_argument('--port',type=int,default=0)
    for command in sub.choices.values(): command.add_argument('--report',required=True)
    args=parser.parse_args()
    try:
        if args.mode=='prepare': result=prepare(args)
        elif args.mode=='check':
            _,manifest=attach(args.fixture_manifest)
            result={'status':'FIXTURE_READY','acceptance_passed':False,'system_id':manifest['identity']['system_id'],'core_database':manifest['core']['database'],'go_database':manifest['go']['database']}
        elif args.mode=='run': result=run(args)
        else:
            endpoint=CountingHTTP(json.loads(private_read(args.script_config)),args.port);endpoint.start()
            print(json.dumps({'status':'COUNTING_FIXTURE_READY','url':f'http://127.0.0.1:{endpoint.server.server_port}'}),flush=True)
            try:
                while True: time.sleep(.5)
            except KeyboardInterrupt: pass
            finally: endpoint.close()
            result={'status':'COUNTING_FIXTURE_ONLY','counts':endpoint.snapshot(),'acceptance_passed':False}
    except (Blocked,FixtureError) as error:
        result={'status':'BLOCKED','reason':str(error),'acceptance_passed':False,'resources_retained':True}
    except (Failed,OSError,ValueError,KeyError,subprocess.TimeoutExpired) as error:
        result={'status':'FAIL','error_type':type(error).__name__,'acceptance_passed':False,'resources_retained':True}
    write_private(Path(args.report),json.dumps(result,indent=2)+'\n')
    print(json.dumps({'status':result['status'],'report':args.report,'reason':result.get('reason')}))
    return 0 if result['status'] in {'PASS','FIXTURE_READY','COUNTING_FIXTURE_ONLY'} else 2 if result['status']=='BLOCKED' else 1

if __name__=='__main__':
    sys.dont_write_bytecode=True
    raise SystemExit(main())
