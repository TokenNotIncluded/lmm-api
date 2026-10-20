"""Offline history and explicit scope-coverage writer.

Input logs and generated material/SQL may contain personal data: a real operator
must keep those files under the private Backups directory, never ordinary output
or Git. Only an explicit offline execute call opens a target connection; there
is no automatic apply CLI or runtime import route.

A complete table/export review and exact identity/credential maps authorize empty
scope receipts. A global source flag or the presence of somebody else's logs
never establishes an empty user's/key's history. Unknown captured pricing remains
NULL. Opaque `other`, secrets and arbitrary metadata never enter history rows or
runtime import receipts. No balances, usage counters or model requests are posted.

Scope hashes are SHA256 of UTF-8 canonical JSON (sorted keys, no insignificant
whitespace, ensure_ascii=False, allow_nan=False). The scope record binds archive,
export/table hashes, source/target identity, key mapping and every original row
hash in source-log-id order, including the explicitly reviewed empty list.
"""
from __future__ import annotations

import copy
import json
from pathlib import Path
import re
import uuid

import financial_import as F

SCOPE = "history-scope-import-v1"
CONTRACT = "legacy-logs-reviewed-v1"
CHANGED_TABLES = ("core_history.sources", "core_history.log_records", "core_history.scope_receipts")
SEQUENCE = "core_history.sources_source_id_seq"
require = F.require
integer = F.PLANNER.integer
MAX_ID = 9007199254740991
MAX_TIME = 253402300799
TEXT_FIELDS = ("username", "token_name", "model_name", "group_name", "request_id", "upstream_request_id")
ROW_FIELDS = frozenset(("source_log_id", "canonical_event_key", "source_row_sha256", "user_id", "owner_account_id",
    "token_id", "created_at", "log_type", "quota", "pricing_credits_per_usd", "prompt_tokens", "completion_tokens",
    "channel_id", *TEXT_FIELDS))


def source_hashes():
    return {**F.source_hashes(), Path(__file__).name: F.sha(Path(__file__).read_bytes())}


def canonical_sha(value):
    return F.sha(F.canonical(value).encode("utf-8"))


class _Number(str):
    pass


def _invalid_json_constant(_):
    raise ValueError("invalid_json_constant")


def captured_basis(metadata):
    """Bounded exact captured anchor, matching the existing trusted Rust oracle.

    This parses the original numeric lexeme; neither binary floats nor the current
    exchange rate can establish a historical price. Invalid metadata is unknown.
    """
    if type(metadata) is not str or len(metadata.encode()) > 65536:
        return None
    try:
        value = json.loads(metadata, parse_int=_Number, parse_float=_Number,
                           parse_constant=_invalid_json_constant, object_pairs_hook=F.PLANNER.no_duplicate_keys)
    except (ValueError, F.Error):
        return None
    raw = value.get("pricing_credits_per_usd") if type(value) is dict else None
    if type(raw) is not _Number or len(raw) > 64 or not raw[0].isdigit():
        return None
    m = re.fullmatch(r"([0-9]+)(?:\.([0-9]+))?(?:[eE]([+-]?[0-9]+))?", raw)
    if m is None:
        return None
    exponent = int(m[3] or "0")
    if not -18 <= exponent <= 18:
        return None
    fraction = (m[2] or "").rstrip("0")
    number = int(m[1] + fraction)
    maximum = 2**128-1
    if number > maximum:
        return None
    shift = exponent-len(fraction)
    factor = 10**abs(shift)
    if factor > maximum:
        return None
    if shift >= 0:
        number *= factor
        if number > maximum:
            return None
    else:
        number, remainder = divmod(number, factor)
        if remainder:
            return None
    return number if 1 <= number <= MAX_ID else None


def _nullable_integer(value, code):
    return None if value is None else integer(value, code)


def _text(value):
    require(type(value) is str and len(value.encode()) <= 256 and "\0" not in value, "history_text_unrepresentable")
    return value


def _bindings(attestation, credential_receipt):
    native = {r["id"]:r["personal_account_id"] for r in attestation["native_users"]}
    require(type(credential_receipt) is dict and credential_receipt.get("scope") == "a02-credential-mapping-receipt"
            and credential_receipt.get("source_archive_sha256") == attestation["source_archive_sha256"]
            and type(credential_receipt.get("records")) is list, "history_credential_receipt_invalid")
    bindings, seen = [], set()
    for r in credential_receipt["records"]:
        require(type(r) is dict, "history_credential_mapping_invalid")
        item = {k:integer(r.get(k), "history_credential_mapping_invalid", minimum=1)
                for k in ("source_key_id", "target_credential_id", "target_user_id", "target_account_id")}
        require(native.get(item["target_user_id"]) == item["target_account_id"], "history_credential_owner_mismatch")
        require(item["source_key_id"] not in seen, "history_credential_mapping_duplicate")
        seen.add(item["source_key_id"])
        bindings.append(item)
    require(len({r["target_credential_id"] for r in bindings}) == len(bindings), "history_credential_mapping_duplicate")
    return native, sorted(bindings, key=lambda r:r["source_key_id"])


def _scope_records(source, rows, identities, bindings):
    scopes = []
    grouped = {}
    for r in rows:
        item = {"source_log_id":r["source_log_id"],"source_row_sha256":r["source_row_sha256"]}
        grouped.setdefault((r["user_id"],r["owner_account_id"],None),[]).append(item)
        if r["token_id"] is not None:
            grouped.setdefault((r["user_id"],r["owner_account_id"],r["token_id"]),[]).append(item)
    for uid, aid in sorted(identities.items()):
        scopes.append({"source_user_id":uid,"user_id":uid,"owner_account_id":aid,
                       "source_key_id":None,"token_id":None})
    for b in bindings:
        scopes.append({"source_user_id":b["target_user_id"],"user_id":b["target_user_id"],
                       "owner_account_id":b["target_account_id"],"source_key_id":b["source_key_id"],
                       "token_id":b["target_credential_id"]})
    for scope in scopes:
        scope.update({k:source[k] for k in ("source_archive_sha256", "source_export_sha256", "source_table_sha256")})
        scope["rows"] = grouped.get((scope["user_id"],scope["owner_account_id"],scope["token_id"]),[])
    return scopes


def build_plan(raw_logs, attestation, credential_receipt, *, source_export_sha256, source_namespace):
    """Build an unapproved candidate from the entire frozen source log table.

    There is no filtering or orphan dropping. Missing users/keys, invalid fields
    and duplicate IDs reject the whole candidate. The reviewer must independently
    verify that `raw_logs` is the full frozen table matching the export hash.
    """
    F.validate_material(attestation)
    require(type(raw_logs) is list, "history_source_table_invalid")
    F.checked_hash(source_export_sha256)
    require(type(source_namespace) is str and 1 <= len(source_namespace) <= 128 and "\0" not in source_namespace,
            "history_source_namespace_invalid")
    identities, bindings = _bindings(attestation, credential_receipt)
    keys = {r["source_key_id"]:r for r in bindings}
    archive = attestation["source_archive_sha256"]
    rows, seen = [], set()
    for raw in raw_logs:
        require(type(raw) is dict, "history_source_row_invalid")
        sid = integer(raw.get("id"), "history_source_id_invalid", minimum=1, maximum=MAX_ID)
        require(sid not in seen, "history_source_id_duplicate")
        seen.add(sid)
        uid = integer(raw.get("user_id"), "history_source_user_invalid", minimum=1)
        require(uid in identities, "history_source_user_unmapped")
        key = integer(raw.get("token_id"), "history_source_key_invalid", minimum=0)
        binding = keys.get(key)
        require(key == 0 or binding is not None, "history_source_key_unmapped")
        require(key == 0 or (binding["target_user_id"] == uid and binding["target_account_id"] == identities[uid]),
                "history_source_key_owner_mismatch")
        r = {"source_log_id":sid,"canonical_event_key":f"legacy.logs:{archive}:{sid}",
             "source_row_sha256":F.digest(raw),"user_id":uid,"owner_account_id":identities[uid],
             "token_id":None if key == 0 else binding["target_credential_id"],
             "created_at":integer(raw.get("created_at"),"history_source_time_invalid",minimum=0,maximum=MAX_TIME),
             "log_type":integer(raw.get("type"),"history_source_type_invalid",minimum=0,maximum=7),
             "quota":_nullable_integer(raw.get("quota"),"history_quota_invalid"),
             "pricing_credits_per_usd":captured_basis(raw.get("other")),
             "prompt_tokens":_nullable_integer(raw.get("prompt_tokens"),"history_tokens_invalid"),
             "completion_tokens":_nullable_integer(raw.get("completion_tokens"),"history_tokens_invalid"),
             "channel_id":integer(raw.get("channel"),"history_channel_invalid",minimum=0)}
        for field in TEXT_FIELDS:
            r[field] = _text(raw.get("group" if field == "group_name" else field))
        rows.append(r)
    rows.sort(key=lambda r:r["source_log_id"])
    source = {"contract_version":"core.history-source.v1","source_archive_sha256":archive,
              "source_export_sha256":source_export_sha256,
              "source_table_sha256":F.digest([{ "source_log_id":r["source_log_id"],
                    "source_row_sha256":r["source_row_sha256"]} for r in rows]),
              "source_namespace":source_namespace,"observed_at":attestation["observation"]["observed_at"],
              "row_count":len(rows),"credential_mapping_sha256":F.digest(credential_receipt)}
    plan = {"format_version":1,"contract":CONTRACT,"source":source,"rows":rows,
            "identities":[{"user_id":u,"owner_account_id":a} for u,a in sorted(identities.items())],
            "credential_bindings":bindings,"scopes":_scope_records(source,rows,identities,bindings)}
    plan["mapping_sha256"] = F.digest(plan)
    validate_plan(plan)
    return plan


def validate_plan(plan):
    require(type(plan) is dict and set(plan) == {"format_version","contract","source","rows","identities",
        "credential_bindings","scopes","mapping_sha256"}, "history_plan_shape_invalid")
    require(type(plan["format_version"]) is int and plan["format_version"] == 1 and plan["contract"] == CONTRACT,
            "history_plan_contract_invalid")
    unsigned = {k:v for k,v in plan.items() if k != "mapping_sha256"}
    require(F.digest(unsigned) == plan["mapping_sha256"], "history_plan_mapping_mismatch")
    source = plan["source"]
    require(type(source) is dict and set(source) == {"contract_version","source_archive_sha256","source_export_sha256",
        "source_table_sha256","source_namespace","observed_at","row_count","credential_mapping_sha256"},
        "history_source_shape_invalid")
    for k in ("source_archive_sha256","source_export_sha256","source_table_sha256","credential_mapping_sha256"):
        F.checked_hash(source[k])
    require(source["contract_version"] == "core.history-source.v1" and type(source["source_namespace"]) is str
            and 1 <= len(source["source_namespace"]) <= 128 and "\0" not in source["source_namespace"],
            "history_source_contract_invalid")
    integer(source["observed_at"],"history_observation_invalid",minimum=1,maximum=MAX_TIME)
    require(type(plan["rows"]) is list and type(plan["identities"]) is list
            and type(plan["credential_bindings"]) is list, "history_mapping_shape_invalid")
    ids = {}
    for i in plan["identities"]:
        require(type(i) is dict and set(i) == {"user_id","owner_account_id"},"history_identity_invalid")
        uid=integer(i["user_id"],"history_identity_invalid",minimum=1)
        aid=integer(i["owner_account_id"],"history_identity_invalid",minimum=1)
        require(uid not in ids and aid not in ids.values(),"history_identity_duplicate")
        ids[uid]=aid
    _, bindings = _bindings({"native_users":[{"id":u,"personal_account_id":a} for u,a in ids.items()],
        "source_archive_sha256":source["source_archive_sha256"]},
        {"scope":"a02-credential-mapping-receipt","source_archive_sha256":source["source_archive_sha256"],
         "records":plan["credential_bindings"]})
    keymap = {b["target_credential_id"]:b for b in bindings}
    seen=[]; seen_ids=set()
    for r in plan["rows"]:
        require(type(r) is dict and set(r) == ROW_FIELDS,"history_row_shape_invalid")
        sid=integer(r["source_log_id"],"history_source_id_invalid",minimum=1,maximum=MAX_ID)
        require(sid not in seen_ids,"history_source_id_duplicate");seen.append(sid);seen_ids.add(sid)
        require(r["canonical_event_key"] == f"legacy.logs:{source['source_archive_sha256']}:{sid}","history_event_identity_mismatch")
        F.checked_hash(r["source_row_sha256"])
        require(type(r["user_id"]) is int and type(r["owner_account_id"]) is int
                and ids.get(r["user_id"]) == r["owner_account_id"],"history_row_owner_mismatch")
        if r["token_id"] is not None:
            integer(r["token_id"],"history_key_invalid",minimum=1)
            b=keymap.get(r["token_id"])
            require(b is not None and b["target_user_id"] == r["user_id"]
                    and b["target_account_id"] == r["owner_account_id"],"history_key_owner_mismatch")
        integer(r["created_at"],"history_time_invalid",minimum=0,maximum=MAX_TIME)
        integer(r["log_type"],"history_type_invalid",minimum=0,maximum=7)
        integer(r["channel_id"],"history_channel_invalid",minimum=0)
        for k in ("quota","prompt_tokens","completion_tokens"):
            _nullable_integer(r[k],"history_counter_invalid")
        if r["pricing_credits_per_usd"] is not None:
            integer(r["pricing_credits_per_usd"],"history_basis_invalid",minimum=1,maximum=MAX_ID)
        for k in TEXT_FIELDS:_text(r[k])
    require(seen == sorted(seen) and type(source["row_count"]) is int and source["row_count"] == len(seen),
            "history_source_row_count_mismatch")
    require(source["source_table_sha256"] == F.digest([{"source_log_id":r["source_log_id"],
            "source_row_sha256":r["source_row_sha256"]} for r in plan["rows"]]), "history_source_rows_mismatch")
    require(plan["scopes"] == _scope_records(source,plan["rows"],ids,bindings), "history_scope_coverage_mismatch")


def prepare_material(attestation, plan, source_review, *, migration_id, source_files):
    m={"format_version":1,"scope":SCOPE,"migration_id":migration_id,"financial_attestation":attestation,
       "plan":plan,"source_review":source_review,"source_files":source_files,"tool_source_hashes":source_hashes()}
    validate_material(m)
    return copy.deepcopy(m)


def validate_material(m):
    require(type(m) is dict and set(m) == {"format_version","scope","migration_id","financial_attestation",
        "plan","source_review","source_files","tool_source_hashes"},"history_material_shape_invalid")
    require(type(m["format_version"]) is int and m["format_version"] == 1 and m["scope"] == SCOPE,
            "history_material_contract_invalid")
    require(type(m["migration_id"]) is str and F.IDENTITY.MIGRATION_ID.fullmatch(m["migration_id"]),"history_migration_id_invalid")
    require(m["tool_source_hashes"] == source_hashes(),"history_tool_changed")
    a=m["financial_attestation"]; F.validate_material(a)
    p=m["plan"];validate_plan(p)
    require(p["source"]["source_archive_sha256"] == a["source_archive_sha256"]
            and p["identities"] == [{"user_id":r["id"],"owner_account_id":r["personal_account_id"]} for r in a["native_users"]]
            and p["source"]["observed_at"] == a["observation"]["observed_at"],"history_attestation_source_mismatch")
    r=m["source_review"]
    require(type(r) is dict and set(r) == {"format_version","scope","decision","source_archive_sha256",
        "source_export_sha256","source_table_sha256","mapping_sha256","identity_map_sha256",
        "credential_mapping_sha256","complete_source_table","complete_identity_and_key_mapping","evidence_sha256"},
        "history_review_shape_invalid")
    require(type(r["format_version"]) is int and r["format_version"] == 1 and r["scope"] == SCOPE
            and r["decision"] == "approved-offline-history-scopes" and r["complete_source_table"] is True
            and r["complete_identity_and_key_mapping"] is True
            and r["identity_map_sha256"] == a["identity_map_sha256"] and r["mapping_sha256"] == p["mapping_sha256"]
            and all(r[k] == p["source"][k] for k in ("source_archive_sha256","source_export_sha256",
                "source_table_sha256","credential_mapping_sha256")),"history_source_coverage_unproven")
    F.checked_hash(r["evidence_sha256"])
    files=m["source_files"]
    require(type(files) is dict and set(files)=={"history_export","credential_mapping","source_review"},
            "history_source_files_incomplete")
    for item in files.values():
        require(type(item) is dict and set(item)=={"path","sha256"} and type(item["path"]) is str
                and Path(item["path"]).is_absolute(),"history_source_file_invalid")
        F.checked_hash(item["sha256"])
    require(files["history_export"]["sha256"]==p["source"]["source_export_sha256"],"history_export_file_binding_mismatch")
    state=a["before_state"]
    require(set(CHANGED_TABLES) <= set(state["inventory"]["tables"])
            and SEQUENCE in state["inventory"]["sequences"],"history_inventory_incomplete")
    require(all(state["tables"][t]["rows"] == 0 for t in CHANGED_TABLES),"history_seed_not_empty")
    seq=state["sequences"][SEQUENCE]
    require(type(seq["increment"]) is int and seq["increment"] == 1
            and type(seq["cache"]) is int and seq["cache"] == 1 and seq["cycle"] is False
            and type(seq["is_called"]) is bool,"history_sequence_noncanonical")
    last=integer(seq["last_value"],"history_sequence_invalid",minimum=1)
    maximum=integer(seq["max"],"history_sequence_invalid",minimum=1)
    require(last+int(seq["is_called"]) <= maximum,"history_sequence_exhausted")


def _envelope(m):
    return {**m["financial_attestation"],"migration_id":m["migration_id"]}


def _quote(sql):
    tag="$lmm_history_"+F.sha(sql.encode())[:24]+"$"
    require(tag not in sql,"history_sql_delimiter_collision")
    prefix,sep,suffix=sql.rpartition("END $lmm_history$;")
    require(bool(sep),"history_sql_template_invalid")
    return prefix.replace("DO $lmm_history$","DO "+tag,1)+"END "+tag+";"+suffix


def build_apply_sql(m):
    validate_material(m)
    e=_envelope(m);p=m["plan"];inv=e["before_state"]["inventory"]
    state=F.state_sql(inv,m["migration_id"]);guard=F._guard(e)
    seq=e["before_state"]["sequences"][SEQUENCE]
    changes="ARRAY["+",".join(F.literal(t) for t in CHANGED_TABLES)+"]::text[]"
    source_cols=("contract_version","source_archive_sha256","source_namespace","observed_at")
    source={k:p["source"][k] for k in source_cols}
    source.update(mapping_sha256=p["mapping_sha256"],complete=True)
    receipts=[{k:s[k] for k in ("user_id","owner_account_id","token_id","source_archive_sha256")}
        | {"mapping_sha256":p["mapping_sha256"],"source_scope_sha256":canonical_sha(s),"complete":True}
        for s in p["scopes"]]
    sql=F._start(e,readonly=False)+f"""
DO $lmm_history$
DECLARE m jsonb := {F.literal(m)}::jsonb; previous core_migration.import_runs%ROWTYPE;
 before_state jsonb; after_state jsonb; source_id_var bigint;
BEGIN
 {guard}
 LOCK TABLE {','.join(F.relation(t) for t in inv['tables'])} IN ACCESS EXCLUSIVE MODE;
 {guard}
 SELECT ({state}) INTO before_state;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->'plan'->'source'->>'source_archive_sha256'
   OR previous.mapping_sha256<>m->'plan'->>'mapping_sha256' OR previous.manifest->'material' IS DISTINCT FROM m
   OR previous.manifest->>'scope' IS DISTINCT FROM {F.literal(SCOPE)}
   OR previous.manifest->'post_state' IS DISTINCT FROM before_state
  THEN RAISE EXCEPTION 'history_replay_conflict'; END IF;
 ELSE
  IF before_state IS DISTINCT FROM m->'financial_attestation'->'before_state'
   OR EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope'={F.literal(SCOPE)})
  THEN RAISE EXCEPTION 'history_target_state_changed'; END IF;
  IF NOT EXISTS(SELECT 1 FROM core_migration.import_runs r WHERE to_jsonb(r)=m->'financial_attestation'->'identity_import')
   OR (SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'personal_account_id',personal_account_id) ORDER BY id),'[]'::jsonb)
       FROM core_identity.users) IS DISTINCT FROM m->'financial_attestation'->'native_users'
  THEN RAISE EXCEPTION 'history_identity_binding_mismatch'; END IF;
  IF EXISTS(SELECT 1 FROM jsonb_array_elements(m->'plan'->'credential_bindings') b
    LEFT JOIN core_identity.credentials c ON c.id=(b->>'target_credential_id')::bigint
    WHERE c.id IS NULL OR c.kind<>'api_key' OR c.user_id IS DISTINCT FROM (b->>'target_user_id')::bigint
     OR c.owner_account_id IS DISTINCT FROM (b->>'target_account_id')::bigint)
  THEN RAISE EXCEPTION 'history_credential_binding_mismatch'; END IF;
  ALTER SEQUENCE {F.relation(SEQUENCE)} RESTART WITH {seq['last_value']+int(seq['is_called'])};
  INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete)
   SELECT contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete
   FROM jsonb_populate_record(NULL::core_history.sources,{F.literal(source)}::jsonb) RETURNING source_id INTO source_id_var;
  INSERT INTO core_history.log_records SELECT * FROM jsonb_populate_recordset(NULL::core_history.log_records,
    (SELECT coalesce(jsonb_agg(value || jsonb_build_object('source_id',source_id_var)),'[]'::jsonb)
     FROM jsonb_array_elements(m->'plan'->'rows')));
  INSERT INTO core_history.scope_receipts SELECT * FROM jsonb_populate_recordset(NULL::core_history.scope_receipts,
    (SELECT coalesce(jsonb_agg(value || jsonb_build_object('source_id',source_id_var)),'[]'::jsonb)
     FROM jsonb_array_elements({F.literal(receipts)}::jsonb)));
  SET CONSTRAINTS ALL IMMEDIATE;
  IF (SELECT to_jsonb(s)-'source_id' FROM core_history.sources s WHERE s.source_id=source_id_var)
      IS DISTINCT FROM {F.literal(source)}::jsonb
   OR (SELECT coalesce(jsonb_agg(to_jsonb(l)-'source_id' ORDER BY l.source_log_id),'[]'::jsonb)
       FROM core_history.log_records l WHERE l.source_id=source_id_var) IS DISTINCT FROM m->'plan'->'rows'
   OR (SELECT coalesce(jsonb_agg(to_jsonb(c)-'source_id' ORDER BY c.user_id,c.token_id NULLS FIRST),'[]'::jsonb)
       FROM core_history.scope_receipts c WHERE c.source_id=source_id_var)
      IS DISTINCT FROM (SELECT coalesce(jsonb_agg(value ORDER BY (value->>'user_id')::bigint,(value->>'token_id')::bigint NULLS FIRST),'[]'::jsonb)
        FROM jsonb_array_elements({F.literal(receipts)}::jsonb))
  THEN RAISE EXCEPTION 'history_exact_readback_failed'; END IF;
  SELECT ({state}) INTO after_state;
  IF (before_state->'tables') - {changes} IS DISTINCT FROM (after_state->'tables') - {changes}
   OR (before_state->'sequences') - {F.literal(SEQUENCE)} IS DISTINCT FROM (after_state->'sequences') - {F.literal(SEQUENCE)}
  THEN RAISE EXCEPTION 'history_unrelated_state_changed'; END IF;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->'plan'->'source'->>'source_archive_sha256',m->'plan'->>'mapping_sha256',
    jsonb_build_object('scope',{F.literal(SCOPE)},'material',m,'post_state',after_state));
 END IF;
 {guard}
END $lmm_history$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{F.literal(m['migration_id'])},'material_sha256',{F.literal(F.digest(m))});
"""
    return _quote(sql)


def verify_approval(m,sql,approval):
    validate_material(m)
    require(type(approval) is dict and set(approval)=={"format_version","scope","decision","material_sha256",
        "sql_sha256","source_review_sha256"} and type(approval["format_version"]) is int and approval["format_version"]==1
        and approval["scope"]==SCOPE and approval["decision"]=="approve-offline-history-scopes"
        and approval["material_sha256"]==F.digest(m) and approval["sql_sha256"]==F.sha(sql.encode())
        and approval["source_review_sha256"]==F.digest(m["source_review"]) and sql==build_apply_sql(m),"history_approval_mismatch")


def build_resolve_sql(m, original_application_name):
    validate_material(m)
    require(type(original_application_name) is str and re.fullmatch(r"lmm-history-[0-9a-f]{32}",original_application_name),
            "history_original_attempt_invalid")
    e=_envelope(m);state=F.state_sql(e["before_state"]["inventory"],m["migration_id"])
    sql=F._start(e,readonly=True)+f"""
DO $lmm_history$
DECLARE m jsonb := {F.literal(m)}::jsonb; previous core_migration.import_runs%ROWTYPE; actual jsonb;
BEGIN
 {F._guard(e)}
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name={F.literal(original_application_name)}
  AND datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid())
 THEN RAISE EXCEPTION 'history_original_attempt_active'; END IF;
 SELECT ({state}) INTO actual;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->'plan'->'source'->>'source_archive_sha256'
   OR previous.mapping_sha256<>m->'plan'->>'mapping_sha256' OR previous.manifest->'material' IS DISTINCT FROM m
   OR previous.manifest->>'scope' IS DISTINCT FROM {F.literal(SCOPE)}
   OR previous.manifest->'post_state' IS DISTINCT FROM actual
  THEN RAISE EXCEPTION 'history_resolution_conflict'; END IF;
 ELSIF actual IS DISTINCT FROM m->'financial_attestation'->'before_state'
 THEN RAISE EXCEPTION 'history_absence_not_proven'; END IF;
 {F._guard(e)}
END $lmm_history$;
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id={F.literal(m['migration_id'])})
 THEN 'COMMITTED' ELSE 'NOT_APPLIED' END,'material_sha256',{F.literal(F.digest(m))});
COMMIT;
"""
    return _quote(sql)


def verify_source_files(m):
    """Rebuild the approved mapping from private local files; never query source."""
    validate_material(m)
    a=m["financial_attestation"]
    F.verify_source_files(a)
    files=m["source_files"]
    loaded={}
    for name,item in files.items():
        path=F.IDENTITY.private_backup_path(Path(item["path"]))
        require(F.IDENTITY.sha256_file(path)==item["sha256"],"history_source_file_changed")
        loaded[name]=F.PLANNER.parse_json(F.read_private(path))
    require(F.digest(loaded["source_review"])==F.digest(m["source_review"]),"history_source_review_file_mismatch")
    require(F.digest(loaded["credential_mapping"])==m["plan"]["source"]["credential_mapping_sha256"],
            "history_credential_file_mismatch")
    rebuilt=build_plan(loaded["history_export"],a,loaded["credential_mapping"],
        source_export_sha256=files["history_export"]["sha256"],source_namespace=m["plan"]["source"]["source_namespace"])
    require(rebuilt==m["plan"],"history_original_export_mapping_mismatch")


def execute(m, *, approval, original_sql, connection, output, original_attempt=None):
    """One explicit submission or original-attempt resolution. Never retries IO.

    All validations before recording an attempt refuse without sending SQL.
    Once the private attempt and exact SQL are durable, every uncertainty returns
    UNKNOWN. The caller must close that original session and use the same
    material/attempt under the session fence to resolve before any new submission.
    """
    validate_material(m)
    require(original_sql==build_apply_sql(m),"history_original_sql_changed")
    verify_approval(m,original_sql,approval)
    verify_source_files(m)
    owner,login=F.IDENTITY.connection_roles(connection)
    target=m["financial_attestation"]["target"]["instance"]
    require(owner==target["role"] and login==target["session_user"]
            and connection.get("database")==target["database"] and connection.get("port")==target["port"],
            "history_target_connection_mismatch")
    F.IDENTITY.socket_connection(connection)
    application_name="lmm-history-"+uuid.uuid4().hex
    attempt={"format_version":1,"migration_id":m["migration_id"],"material_sha256":F.digest(m),
        "sql_sha256":F.sha(original_sql.encode()),"application_name":application_name,
        "operation":"resolve" if original_attempt is not None else "apply"}
    sql=original_sql
    if original_attempt is not None:
        require(type(original_attempt) is dict and original_attempt.get("operation")=="apply"
                and all(original_attempt.get(k)==attempt[k] for k in ("migration_id","material_sha256","sql_sha256")),
                "history_original_attempt_material_mismatch")
        sql=build_resolve_sql(m,original_attempt.get("application_name"))
    F.write_private(output,"attempt.private.json",F.canonical(attempt)+"\n")
    F.write_private(output,"executed.private.sql",sql)
    outcome={"status":"UNKNOWN","migration_id":m["migration_id"],"material_sha256":F.digest(m)}
    try:
        code,stdout=F.run_sql(connection,sql,output,application_name)
        if code==0:
            verify_source_files(m)
            require(source_hashes()==m["tool_source_hashes"],"history_tool_changed")
            result=F.PLANNER.parse_json(stdout)
            allowed=("COMMITTED","NOT_APPLIED") if original_attempt is not None else ("COMMITTED",)
            if type(result) is dict and set(result)==set(outcome) and result.get("status") in allowed \
                    and all(result.get(k)==outcome[k] for k in ("migration_id","material_sha256")):
                outcome=result
    except Exception:
        pass
    try:
        F.write_private(output,"result.private.json",F.canonical(outcome)+"\n")
    except Exception:
        outcome={**outcome,"status":"UNKNOWN"}
    return outcome
