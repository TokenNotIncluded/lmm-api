#!/usr/bin/env python3
"""Restore the complete compiled Core into a new, owned synthetic PostgreSQL.

This runner accepts no DSN, existing database, container or data directory.
Backups and keys remain in Backups. A failed/uncertain mutation is retained.
It is an offline qualification tool, not an online upgrade or holder takeover.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import stat
import urllib.parse

import backup
from pg_fixture import Fixture, FixtureError, identifier, require_tools, write_private

ROOT = Path(__file__).resolve().parents[2]
NS = "n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'"


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def checked_file(path, expected):
    if len(expected) != 64 or any(c not in '0123456789abcdef' for c in expected):
        raise FixtureError('a reviewed SHA-256 is required')
    if path.resolve() != path or not path.is_file() or path.is_symlink():
        raise FixtureError('a canonical regular input is required')
    info=path.stat()
    if info.st_uid not in {0,os.getuid()} or stat.S_IMODE(info.st_mode)&0o022:
        raise FixtureError('frozen input has an untrusted owner or writable mode')
    if sha(path.read_bytes()) != expected:
        raise FixtureError('frozen input changed')


def invoke(binary, arguments, env):
    try:
        result = subprocess.run([str(binary), *arguments], env=env,
                                capture_output=True, timeout=120)
    except (OSError, subprocess.TimeoutExpired):
        raise FixtureError('Core command result UNKNOWN; retained fixture, no retry') from None
    if result.returncode:
        # Config, driver output and SQL can contain credentials or user data.
        raise FixtureError('Core command failed; retained fixture, no retry')
    return result.stdout


def catalog_fingerprint(lines):
    h = hashlib.sha256()
    for line in lines:
        raw = line.encode('utf8')
        h.update(len(raw).to_bytes(8, 'big'))
        h.update(raw)
    return h.hexdigest()


def snapshot(fixture, database, catalog_query):
    """Hash every row, sequence, owner and ACL without publishing row contents.

    Both login writers have already been fenced and other clients excluded.
    Each query is read only. No guessed schema/table allowlist omits new domains.
    """
    def read(query):
        return fixture.sql('BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; '
                           "SET LOCAL search_path=pg_catalog; " + query + '; COMMIT;', database)
    inventory = json.loads(read(f"SELECT coalesce(json_agg(json_build_array(n.nspname,c.relname,c.relkind) "
        f"ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"),'[]'::json) "
        f"FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {NS} AND c.relkind IN ('r','p','S')"))
    tables, sequences = {}, {}
    for schema, name, kind in inventory:
        # Names come from the server, not user SQL. Standard identifier quoting.
        quoted = '.'.join('"' + part.replace('"', '""') + '"' for part in (schema, name))
        if kind == 'S':
            sequences[schema + '.' + name] = read('SELECT last_value,is_called FROM ' + quoted)
        else:
            raw = read('SELECT row_to_json(t)::text FROM ' + quoted + ' t ORDER BY row_to_json(t)::text COLLATE "C"')
            tables[schema + '.' + name] = sha(raw.encode())
    acl_query = f"""WITH objects(kind,name,owner,acl) AS (
      SELECT 'schema',nspname,nspowner,coalesce(nspacl,acldefault('n',nspowner)) FROM pg_namespace n WHERE {NS}
      UNION ALL SELECT 'relation',n.nspname||'.'||c.relname,c.relowner,
        coalesce(c.relacl,acldefault(CASE WHEN c.relkind='S' THEN 's'::\"char\" ELSE 'r'::\"char\" END,c.relowner))
        FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {NS} AND c.relkind IN ('r','p','S','v','m','f')
      UNION ALL SELECT 'function',n.nspname||'.'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')',p.proowner,
        coalesce(p.proacl,acldefault('f',p.proowner)) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE {NS}
      UNION ALL SELECT 'column',n.nspname||'.'||c.relname||'.'||a.attname,c.relowner,a.attacl
        FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace
        WHERE {NS} AND a.attnum>0 AND NOT a.attisdropped AND a.attacl IS NOT NULL
    ) SELECT json_build_array(kind,name,pg_get_userbyid(owner),CASE WHEN acl IS NULL THEN NULL ELSE
      (SELECT json_agg(json_build_array(pg_get_userbyid(x.grantor),CASE WHEN x.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(x.grantee) END,x.privilege_type,x.is_grantable)
       ORDER BY x.grantor,x.grantee,x.privilege_type COLLATE "C",x.is_grantable) FROM aclexplode(acl) x) END)::text
      FROM objects ORDER BY kind COLLATE "C",name COLLATE "C"
    """
    definitions = read(catalog_query).splitlines()
    return {'tables': tables, 'sequences': sequences,
            'catalog_sha256': catalog_fingerprint(definitions),
            'owner_acl_sha256': sha(read(acl_query).encode())}


def run(binary, binary_sha, source_manifest, manifest_sha, private_root):
    checked_file(binary, binary_sha)
    with binary.open('rb') as stream:
        if stream.read(4)!=b'\x7fELF' or not os.access(binary,os.X_OK):
            raise FixtureError('a frozen executable Core ELF is required')
    checked_file(source_manifest, manifest_sha)
    backup.in_backups(private_root / 'reserved')
    require_tools()
    fixture = Fixture(private_root)
    fixture.start()
    completed = False
    try:
        selected = fixture.allocate('A09')
        database, owner, runtime = (selected[k] for k in ('database', 'owner', 'runtime'))
        with backup.private_input(fixture.root/'A09'/'owner.pgpass') as stream:
            password = stream.read(4096).decode().strip().split(':')[-1]
        secret = fixture.root/'owner-url'
        user = urllib.parse.quote(selected['installer'], safe='')
        url = (f'postgresql://{user}:{urllib.parse.quote(password,safe="")}@127.0.0.1:{selected["port"]}/{database}'
               f'?options=-c%20role%3D{owner}')
        write_private(secret, url + '\n')
        config = fixture.root/'owner.toml'
        write_private(config, f'version=1\n[database]\nurl_file={json.dumps(str(secret))}\nmax_connections=1\n')
        compiled_raw = invoke(binary, ['schema-manifest'], fixture.env)
        compiled = json.loads(compiled_raw)
        if compiled['scope'] != 'compiled-core-schema-contract':
            raise FixtureError('compiled full Core manifest required')
        query = compiled['catalog']['query_utf8']
        if sha(query.encode()) != compiled['catalog']['query_sha256']:
            raise FixtureError('compiled catalog query digest mismatch')
        fixture.record('frozen-inputs', {'binary_sha256': binary_sha, 'source_manifest_sha256': manifest_sha,
                                       'compiled_schema_manifest_sha256': sha(compiled_raw)})
        fixture.record('core-init-intent',{'database':database,'system_id':fixture.identity['system_id'],
                                         'binary_sha256':binary_sha,'replay_authorized':False})
        invoke(binary, ['--config', str(config), 'init-db'], fixture.env)
        fixture.record('core-init-acknowledged',{'database':database})
        checked_file(binary, binary_sha)
        checked_file(source_manifest, manifest_sha)
        # Provision only this fixture's role, using the repository's exact ACL contract.
        grant_path = ROOT/'deployment/postgres/grant-core-runtime.sql'
        grant_raw = grant_path.read_bytes()
        grant = grant_raw.decode().replace("'lmm_core_owner'", "'" + owner + "'")
        fixture.sql('\\set runtime_role ' + runtime + '\n' + grant, database)
        fixture.sql('INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES (1001,\'personal\'); '
                    'INSERT INTO core_identity.users(id,personal_account_id) VALUES (1001,1001);', database)
        # All non-maintenance writers are fenced before snapshot/dump. Never kill clients.
        fixture.sql(f'ALTER ROLE {runtime} NOLOGIN; ALTER ROLE {selected["installer"]} NOLOGIN;')
        fixture.verify()
        if fixture.sql(f"SELECT count(*) FROM pg_stat_activity WHERE datname='{database}';") != '0':
            raise FixtureError('source has a client; retained without termination')
        before = snapshot(fixture, database, query)
        target = fixture.restore_database('A09')
        binding = sha(json.dumps({'nonce':fixture.nonce, 'system_id':fixture.identity['system_id'],
            'source':database, 'target':target, 'binary_sha256':binary_sha, 'source_manifest_sha256':manifest_sha,
            'snapshot':before},sort_keys=True).encode())
        keys = fixture.root/'keys'
        keys.mkdir(mode=0o700)
        key = keys/'recovery.key'
        backup.create_key(key)
        plain, sealed, recovered = (fixture.root/name for name in ('core.dump','core.aesgcm','authenticated.dump'))
        fixture.dump(database, plain)
        encrypted = backup.encrypt(plain, sealed, key, binding)
        authenticated = backup.decrypt(sealed, recovered, key, binding)
        fixture.record('approved-new-target', {'binding_sha256':binding,'target':target,
            'system_id':fixture.identity['system_id'],'snapshot_sha256':sha(json.dumps(before,sort_keys=True).encode()),
            'archive_sha256':authenticated['plaintext_sha256'],'runtime_fenced':True})
        try:
            fixture.restore(target, recovered, authenticated['plaintext_sha256'])
        except BaseException:
            # A lost client ACK cannot prove whether the single transaction
            # committed. Never authorize retry from a dead PID or timeout.
            fixture.record('restore-unresolved',{'target':target,'state':'UNKNOWN',
                'binding_sha256':binding,'automatic_replay':False})
            raise
        if snapshot(fixture,target,query) != before or snapshot(fixture,database,query) != before:
            raise FixtureError('full Core restoration mismatch; retained without replay')
        try:
            fixture.restore(target,recovered,authenticated['plaintext_sha256'])
        except FixtureError:
            pass
        else:
            raise FixtureError('occupied target accepted replay')
        # Start two separate real readonly Core check-db processes against the
        # restored DB as its low-authority runtime, never the installation owner.
        with backup.private_input(fixture.root/'A09'/'runtime.pgpass') as stream:
            runtime_password=stream.read(4096).decode().strip().split(':')[-1]
        runtime_url=fixture.root/'restored-runtime-url'
        write_private(runtime_url,f'postgresql://{runtime}:{urllib.parse.quote(runtime_password,safe="")}@127.0.0.1:{selected["port"]}/{target}\n')
        runtime_config=fixture.root/'restored-runtime.toml'
        write_private(runtime_config,f'version=1\n[database]\nurl_file={json.dumps(str(runtime_url))}\nmax_connections=1\n')
        fixture.sql(f'ALTER ROLE {runtime} LOGIN; GRANT CONNECT ON DATABASE {identifier(target)} TO {identifier(runtime)};')
        for _ in range(2):
            observation=json.loads(invoke(binary,['--config',str(runtime_config),'check-db'],fixture.env))
            if observation.get('status')!='validated' or observation.get('compiled_schema_manifest_sha256')!=sha(compiled_raw):
                raise FixtureError('restored runtime validation did not match frozen Core')
        fixture.sql(f'ALTER ROLE {runtime} NOLOGIN;')
        if snapshot(fixture,target,query)!=before or snapshot(fixture,database,query)!=before:
            raise FixtureError('readonly runtime validation changed source or restored state')
        checked_file(binary,binary_sha)
        checked_file(source_manifest,manifest_sha)
        if grant_path.read_bytes()!=grant_raw:
            raise FixtureError('runtime ACL input changed during qualification')
        report = {'format':'core-isolated-recovery-v1','status':'PASS','scope':'fresh synthetic full compiled Core only',
            'binary_sha256':binary_sha,'source_manifest_sha256':manifest_sha,'binding_sha256':binding,
            'runtime_acl_source_sha256':sha(grant_raw),'python':sys.version.split()[0],
            'postgres':fixture.identity['version'],
            'components':len(compiled['components']),'tables':len(before['tables']),'sequences':len(before['sequences']),
            'snapshot_sha256':sha(json.dumps(before,sort_keys=True).encode()),'backup':encrypted,
            'verified':['all_table_rows','all_sequence_values','compiled_catalog','owners_and_ACLs','encrypted_key_reload',
                        'independent_empty_target','source_unchanged','occupied_target_rejected','two_real_runtime_check_db_processes'],
            'not_verified':['online_writer_fence','dead_holder_CAS','production_recovery','runtime_business_restart']}
        fixture.record('full-core-recovery',report)
        completed=True
        return report
    finally:
        # Preserve all owned DBs/keys even on success for independent inspection.
        fixture.record('qualification-ended',{'completed':completed,'resources_retained':True,
                                              'on_failure':'inspect original intent; never replay UNKNOWN'})


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['binary','source-manifest','private-root']:
        parser.add_argument('--'+name,type=Path,required=True)
    for name in ['binary','source-manifest']:
        parser.add_argument('--expect-'+name+'-sha256',required=True)
    args=parser.parse_args(argv)
    try:
        report=run(args.binary,args.expect_binary_sha256,args.source_manifest,
                   args.expect_source_manifest_sha256,args.private_root)
        print(json.dumps(report,sort_keys=True))
        return 0
    except (FixtureError,backup.BackupError,OSError,KeyError,ValueError):
        print(json.dumps({'format':'core-isolated-recovery-v1','status':'BLOCKED_OR_FAILED',
                          'resources_retained':True,'automatic_replay':False}))
        return 3


if __name__=='__main__':
    raise SystemExit(main())
