#!/usr/bin/env python3
"""Explicit disposable PostgreSQL restore probe. No caller DSN or existing DB.

This installs a small synthetic contract, NOT the complete Core schema. It
qualifies fixture ownership, dump authentication, independent empty-target
restore, sequences, SQL functions, triggers and runtime privileges. It cannot
qualify Core maintenance, product restart, pgroll, or production deployment.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import socket
import tempfile

import backup
from pg_fixture import Fixture, FixtureError, PermissionDenied, require_tools


def seed(owner: str, runtime: str) -> str:
    return f'''
SET ROLE {owner};
CREATE SCHEMA a09_probe AUTHORIZATION {owner};
CREATE TABLE a09_probe.entries (
 id BIGSERIAL PRIMARY KEY,
 request_key TEXT NOT NULL UNIQUE,
 credits BIGINT NOT NULL CHECK (credits>0)
);
CREATE TABLE a09_probe.contract (name TEXT PRIMARY KEY, value TEXT NOT NULL);
INSERT INTO a09_probe.contract VALUES ('unit','synthetic_integer_credits'),('schema','a09-probe-v1');
INSERT INTO a09_probe.entries(request_key,credits) VALUES ('request-1',101),('request-2',202),('request-3',303);
CREATE FUNCTION a09_probe.immutable_entry() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN RAISE EXCEPTION 'append only'; END $$;
CREATE TRIGGER append_only BEFORE UPDATE OR DELETE ON a09_probe.entries
FOR EACH ROW EXECUTE FUNCTION a09_probe.immutable_entry();
CREATE FUNCTION a09_probe.record_entry(k TEXT,v BIGINT) RETURNS VOID
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF v<1 OR v>1000 THEN RAISE EXCEPTION 'invalid synthetic credits'; END IF;
 INSERT INTO a09_probe.entries(request_key,credits) VALUES (k,v) ON CONFLICT(request_key) DO NOTHING;
END $$;
REVOKE ALL ON SCHEMA a09_probe FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA a09_probe FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA a09_probe FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA a09_probe FROM PUBLIC;
GRANT USAGE ON SCHEMA a09_probe TO {runtime};
GRANT SELECT ON ALL TABLES IN SCHEMA a09_probe TO {runtime};
GRANT EXECUTE ON FUNCTION a09_probe.record_entry(TEXT,BIGINT) TO {runtime};
RESET ROLE;
'''


SNAPSHOT = '''
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path=pg_catalog;
WITH acl_objects(kind,name,owner,acl) AS (
 SELECT 'relation',c.relname::text,c.relowner,
   coalesce(c.relacl,CASE
     WHEN c.relkind='S' THEN acldefault('s',c.relowner)
     WHEN c.relkind IN ('r','p','v','m','f') THEN acldefault('r',c.relowner)
     ELSE NULL::aclitem[] END)
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='a09_probe'
 UNION ALL
 SELECT 'column',c.relname||'.'||a.attnum||':'||a.attname,c.relowner,
   coalesce(a.attacl,acldefault('c',c.relowner))
 FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid
 JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='a09_probe' AND a.attnum>0 AND NOT a.attisdropped
 UNION ALL
 SELECT 'function',p.proname||'('||pg_get_function_identity_arguments(p.oid)||')',
   p.proowner,coalesce(p.proacl,acldefault('f',p.proowner))
 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname='a09_probe' AND p.prokind='f'
 UNION ALL
 SELECT 'schema',nspname,nspowner,coalesce(nspacl,acldefault('n',nspowner))
 FROM pg_namespace WHERE nspname='a09_probe'
), permissions AS (
 SELECT o.kind,o.name,pg_get_userbyid(o.owner) AS owner,
   coalesce(json_agg(json_build_array(
     pg_get_userbyid(a.grantor),a.grantee=0,
     CASE WHEN a.grantee=0 THEN NULL ELSE pg_get_userbyid(a.grantee) END,
     a.privilege_type,a.is_grantable)
     ORDER BY pg_get_userbyid(a.grantor) COLLATE "C",a.grantee=0,
       pg_get_userbyid(a.grantee) COLLATE "C",a.privilege_type COLLATE "C",a.is_grantable)
     FILTER (WHERE a.privilege_type IS NOT NULL),'[]'::json) AS privileges
 FROM acl_objects o LEFT JOIN LATERAL aclexplode(o.acl) a ON true
 GROUP BY o.kind,o.name,o.owner
)
SELECT json_build_object(
 'rows',(SELECT json_agg(t ORDER BY id) FROM a09_probe.entries t),
 'contract',(SELECT json_agg(t ORDER BY name) FROM a09_probe.contract t),
 'sequence',(SELECT json_build_array(last_value,is_called) FROM a09_probe.entries_id_seq),
 'sequence_definitions',(SELECT json_agg(json_build_array(c.relname,
   format_type(s.seqtypid,NULL),s.seqstart,s.seqincrement,s.seqmin,s.seqmax,
   s.seqcache,s.seqcycle) ORDER BY c.relname COLLATE "C")
   FROM pg_sequence s JOIN pg_class c ON c.oid=s.seqrelid
   JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='a09_probe'),
 'columns',(SELECT json_agg(json_build_array(c.relname,a.attnum,a.attname,
   format_type(a.atttypid,a.atttypmod),a.attnotnull,pg_get_expr(d.adbin,d.adrelid)) ORDER BY c.relname,a.attnum)
   FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid
   JOIN pg_namespace n ON n.oid=c.relnamespace
   LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
   WHERE n.nspname='a09_probe' AND a.attnum>0 AND NOT a.attisdropped),
 'relations',(SELECT json_agg(json_build_array(c.relname,c.relkind,
   pg_get_userbyid(c.relowner)) ORDER BY c.relname)
   FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='a09_probe'),
 'functions',(SELECT json_agg(json_build_array(pg_get_functiondef(p.oid),
   pg_get_userbyid(p.proowner)) ORDER BY p.proname,pg_get_function_identity_arguments(p.oid))
   FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
   WHERE n.nspname='a09_probe' AND p.prokind='f'),
 'indexes',(SELECT json_agg(pg_get_indexdef(i.indexrelid) ORDER BY c.relname)
   FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid
   JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='a09_probe'),
 'triggers',(SELECT json_agg(json_build_array(pg_get_triggerdef(t.oid,true),t.tgenabled) ORDER BY t.tgname)
   FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid
   JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='a09_probe' AND NOT t.tgisinternal),
 'schema_owner',(SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname='a09_probe'),
 'permissions',(SELECT json_agg(json_build_array(kind,name,owner,privileges)
   ORDER BY kind COLLATE "C",name COLLATE "C",owner COLLATE "C") FROM permissions)
);
COMMIT;
'''


def snapshot(fixture: Fixture, database: str) -> dict:
    return json.loads(fixture.sql(SNAPSHOT, database))


def expect_denied(fixture, statement, database, role, password):
    try:
        fixture.sql(statement, database, role=role, password=password)
    except PermissionDenied:
        return
    raise AssertionError('the tested runtime operation was not denied with SQLSTATE 42501')


def run(parent: Path) -> dict:
    backup.in_backups(parent/'fixture-root')  # Includes retained recovery credentials.
    require_tools()  # A missing daemon returns BLOCKED, not a skipped/pass test.
    fixture = Fixture(parent)
    fixture.start()
    completed = False
    directory = None
    try:
        resources = {f'A{i:02d}': fixture.allocate(f'A{i:02d}') for i in range(1, 11)}
        selected = resources['A09']
        source = selected['database']
        role, owner = selected['runtime'], selected['owner']
        with backup.private_input(fixture.root/'A09'/'runtime.pgpass') as stream:
            password = stream.read(4096).decode().strip().split(':')[-1]
        fixture.sql(seed(owner, role), source)
        expect_denied(fixture, f'SET ROLE {owner};', source, role, password)
        fixture.expect_connect_denied(resources['A01']['database'], role, password)
        expect_denied(fixture, "INSERT INTO a09_probe.entries(request_key,credits) VALUES ('unapproved',1);", source, role, password)
        before = snapshot(fixture, source)
        target = fixture.restore_database('A09')
        # Fence the two test login writers. Existing sessions must also be absent.
        fixture.sql(f'ALTER ROLE {role} NOLOGIN; ALTER ROLE {selected["installer"]} NOLOGIN;')
        if fixture.sql(f"SELECT count(*) FROM pg_stat_activity WHERE datname='{source}';") != '0':
            raise FixtureError('source still has a test writer')
        backup.BACKUPS.mkdir(mode=0o700, parents=True, exist_ok=True)
        backup.private_dir(backup.BACKUPS)
        host = ''.join(c if c.isalnum() or c in '-.' else '_' for c in socket.gethostname())
        prefix = f'api-lmm-a09-{host}-{datetime.now(timezone.utc):%Y%m%d}-'
        # The already-validated task root keeps all recovery material bounded.
        directory = Path(tempfile.mkdtemp(prefix=prefix, dir=parent))
        keys = directory/'keys'
        keys.mkdir(mode=0o700)
        key = keys/'recovery.key'
        backup.create_key(key)
        plain, encrypted, recovered = (directory/name for name in ('source.dump','source.aesgcm','recovered.dump'))
        binding = hashlib.sha256(json.dumps({'nonce':fixture.nonce, 'system_id':fixture.identity['system_id'],
                                            'source':source,'target':target},sort_keys=True).encode()).hexdigest()
        fixture.dump(source, plain)
        sealed = backup.encrypt(plain, encrypted, key, binding)
        # Reload the private key, authenticate all data, and only then restore.
        authenticated = backup.decrypt(encrypted, recovered, key, binding)
        fixture.restore(target, recovered, authenticated['plaintext_sha256'])
        after = snapshot(fixture, target)
        if before != after or before != snapshot(fixture, source):
            raise AssertionError('rows, sequence, contract, definitions or permissions changed')
        fixture.sql(f'ALTER ROLE {role} LOGIN; GRANT CONNECT ON DATABASE {target} TO {role};')
        expect_denied(fixture, f'SET ROLE {owner};', target, role, password)
        expect_denied(fixture, "INSERT INTO a09_probe.entries(request_key,credits) VALUES ('unapproved',1);", target, role, password)
        fixture.sql("SELECT a09_probe.record_entry('restored-request',404); SELECT a09_probe.record_entry('restored-request',404);",
                    target, role=role, password=password)
        actual = fixture.sql('SELECT count(*),sum(credits) FROM a09_probe.entries;', target)
        if actual != '4|1010' or snapshot(fixture, source) != before:
            raise AssertionError('post-restore permission/idempotency or original-source invariant failed')
        try:
            fixture.restore(target, recovered, authenticated['plaintext_sha256'])
        except FixtureError:
            pass
        else:
            raise AssertionError('occupied target accepted a second restore')
        report = {'format':'a09-postgres-recovery-probe-v1','status':'PASS',
                  'scope':'synthetic PostgreSQL fixture only; NOT full Core maintenance',
                  'postgres':fixture.identity['version'],'databases_allocated':10,
                  'independent_restore_databases':1,'backup':sealed,
                  'snapshot_sha256':hashlib.sha256(json.dumps(before,sort_keys=True).encode()).hexdigest(),
                  'verified':['source_unchanged','rows','sequence','contract','columns','owners','ACLs',
                              'SECURITY_DEFINER','append_only_trigger','runtime_denials','idempotency'],
                  'not_verified':['Core schema','global role recreation','DB dead-holder CAS','pgroll','product restart']}
        fixture.record('restore-qualification',report)
        completed = True
        return report
    finally:
        if completed:
            # Successful probes remove only their verified, idle owned cluster.
            # Backup and key files remain available for inspection.
            fixture.cleanup()
        else:
            fixture.record('test-failed', {
                'status': 'FAIL', 'fixture_retained': True,
                'backup_retained': directory is not None,
                'backup_status': 'partial_or_complete_requires_inspection',
            })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--private-root',type=Path,required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(run(args.private_root),sort_keys=True))
        return 0
    except (FixtureError, backup.BackupError) as error:
        print(str(error))
        return 2 if str(error).startswith('BLOCKED:') else 1


if __name__ == '__main__':
    raise SystemExit(main())
