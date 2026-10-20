"""Offline subscription writer for a reviewed, UTC-only legacy source contract.

This library has no target connection or apply CLI. It generates an approved SQL
batch for a newly isolated target, using the financial writer's full target
attestation, all-table/sequence CAS, offline roles, session fence and strict
replay rules. Source approval remains separate from SQL approval. Unknown source
units, timezone, reset basis or policy effects reject the entire batch.

A financial opening attestation supplies target/source identity, not permission
for subscriptions. Its wallet payloads are validated but never executed here.
Renewal beyond the recorded expiry is a separate grant. Group transitions and
wallet overflow are unsupported and must be absent, never silently discarded.
Account funding preferences are conserved; importing entitlements does not change
an identity-only target's wallet_only policy or activate subscription spending.
"""
from __future__ import annotations

import copy
from pathlib import Path
import re

import financial_import as F

SCOPE = "subscription-opening-only-v1"
SOURCE_CONTRACT = "legacy-subscription-utc-reviewed-v1"
CHANGED_TABLES = ("core_charging.subscriptions", "core_charging.subscription_opening_consumption")
SEQUENCE = "core_charging.subscriptions_id_seq"
MAX_TIME = 253402300799
require = F.require
integer = F.PLANNER.integer


def source_hashes():
    return {**F.source_hashes(), Path(__file__).name: F.sha(Path(__file__).read_bytes())}


def _policy_list(value):
    require(type(value) is list and value
            and all(type(v) is str and 0 < len(v.encode()) <= 256 and "\0" not in v for v in value)
            and value == sorted(set(value)),
            "subscription_policy_list_invalid")


def build_records(plan, review):
    """Map only source facts whose exact monetary and policy meaning was reviewed.

    `review.policies` must name every source subscription exactly once, including
    inactive grants. Review is an externally approved input, never synthesized
    from current prices or runtime defaults. Original source facts remain in the
    receipt through `financial_attestation.source_plan`.
    """
    F.PLANNER.verify_plan_mapping(plan)
    require(type(review) is dict and set(review) == {
        "format_version", "scope", "source_contract", "decision", "source_archive_sha256",
        "mapping_sha256", "identity_map_sha256", "evidence_sha256", "credits_per_usd",
        "source_timezone", "renewal_policy", "policies"}, "subscription_review_shape_invalid")
    require(type(review["format_version"]) is int and review["format_version"] == 1
            and review["scope"] == SCOPE and review["source_contract"] == SOURCE_CONTRACT
            and review["decision"] == "approved-offline-subscriptions"
            and type(review["credits_per_usd"]) is int and review["credits_per_usd"] == 500000
            and review["source_timezone"] == plan.get("source_timezone") == "UTC"
            and review["renewal_policy"] == "separate-grant-after-expiry",
            "subscription_source_basis_unproven")
    for k in ("source_archive_sha256", "mapping_sha256", "identity_map_sha256"):
        require(review[k] == plan.get(k), "subscription_review_source_mismatch")
    F.checked_hash(review["evidence_sha256"])
    require(type(review["policies"]) is list, "subscription_policy_shape_invalid")
    policies = {}
    for p in review["policies"]:
        require(type(p) is dict and set(p) == {"source_subscription_id", "models", "groups",
            "source_row_sha256", "plan_row_sha256"}, "subscription_policy_shape_invalid")
        sid = integer(p["source_subscription_id"], "subscription_id_invalid", minimum=1)
        require(sid not in policies, "subscription_policy_duplicate")
        _policy_list(p["models"])
        _policy_list(p["groups"])
        policies[sid] = p
    source = plan.get("subscription_entitlements")
    require(type(source) is list and source, "subscription_source_empty")
    require({r["source_subscription_id"] for r in source} == set(policies)
            and len(source) == len(policies), "subscription_policy_coverage_mismatch")
    records = []
    periods = {"daily": "day", "weekly": "week", "monthly": "month", "custom": "custom", "never": "grant"}
    for r in source:
        require(not r["blockers"], "subscription_source_blocked")
        p = policies[r["source_subscription_id"]]
        require(all(p[k] == r[k] for k in ("source_row_sha256", "plan_row_sha256")),
                "subscription_policy_source_mismatch")
        require(r["allow_wallet_overflow"] is False
                and all(r[k] in (None, "") for k in ("upgrade_group", "previous_group", "downgrade_group")),
                "subscription_runtime_policy_unrepresentable")
        require(r["calendar_period"] in periods, "subscription_period_unproven")
        period = periods[r["calendar_period"]]
        start = integer(r["start_time"], "subscription_start_invalid", minimum=0, maximum=MAX_TIME)
        end = integer(r["end_time"], "subscription_end_invalid", minimum=start + 1, maximum=MAX_TIME)
        used = integer(r["opening_used_units"], "subscription_used_invalid", minimum=0)
        amount = integer(r["original_amount_total"], "subscription_limit_invalid", minimum=0)
        unlimited = r["unlimited"]
        require(type(unlimited) is bool, "subscription_unlimited_invalid")
        seconds = 0
        if period == "grant":
            require(r["next_reset_time"] == 0 and r["last_reset_time"] in (0, start),
                    "subscription_never_window_unproven")
            opening_start, opening_end = start, end
            reset_basis, reset_limit, reset_unlimited = "same", 0, False
        else:
            opening_start = max(start, integer(r["last_reset_time"], "subscription_reset_invalid", minimum=0))
            opening_end = min(end, integer(r["next_reset_time"], "subscription_reset_invalid", minimum=opening_start + 1))
            require(start <= opening_start < opening_end <= end, "subscription_opening_window_invalid")
            # A missing reset amount cannot be replaced with today's grant cap.
            reset_unlimited = unlimited and r["reset_amount"] is None
            reset_limit = 0 if reset_unlimited else integer(r["reset_amount"], "subscription_reset_basis_unproven", minimum=0)
            reset_basis = "explicit"
            if period == "custom":
                seconds = integer(r["reset_custom_seconds"], "subscription_custom_period_invalid", minimum=1, maximum=315576000)
        records.append({"source_subscription_id": r["source_subscription_id"],
            "source_user_id": r["source_user_id"], "account_id": r["account_id"],
            "source_row_sha256": r["source_row_sha256"], "plan_row_sha256": r["plan_row_sha256"],
            "period": period, "timezone": "UTC", "anchor": start, "seconds": seconds, "ends_at": end,
            "limit_credits": 0 if unlimited else amount, "unlimited": unlimited,
            "reset_basis": reset_basis, "reset_limit_credits": reset_limit, "reset_unlimited": reset_unlimited,
            "models": p["models"], "groups": p["groups"], "active": r["status"] == "active",
            "opening_start": opening_start, "opening_end": opening_end, "opening_used": used})
    return sorted(records, key=lambda r: r["source_subscription_id"])


def prepare_material(financial_attestation, review, *, migration_id):
    F.validate_material(financial_attestation)
    require(type(migration_id) is str and F.IDENTITY.MIGRATION_ID.fullmatch(migration_id)
            and len(migration_id) <= 100, "subscription_migration_id_invalid")
    records = build_records(financial_attestation["source_plan"], review)
    m = {"format_version": 1, "scope": SCOPE, "migration_id": migration_id,
         "financial_attestation": financial_attestation, "source_review": review,
         "records": records, "mapping_sha256": F.digest(records), "tool_source_hashes": source_hashes()}
    validate_material(m)
    return copy.deepcopy(m)


def validate_material(m):
    require(type(m) is dict and set(m) == {"format_version", "scope", "migration_id", "financial_attestation",
        "source_review", "records", "mapping_sha256", "tool_source_hashes"}, "subscription_material_shape_invalid")
    require(type(m["format_version"]) is int and m["format_version"] == 1 and m["scope"] == SCOPE,
            "subscription_material_contract_invalid")
    require(type(m["migration_id"]) is str and F.IDENTITY.MIGRATION_ID.fullmatch(m["migration_id"])
            and len(m["migration_id"]) <= 100, "subscription_migration_id_invalid")
    a = m["financial_attestation"]
    F.validate_material(a)
    require(m["tool_source_hashes"] == source_hashes(), "subscription_tool_changed")
    require(F.digest(m["records"]) == m["mapping_sha256"]
            and m["records"] == build_records(a["source_plan"], m["source_review"]), "subscription_adapter_mismatch")
    s = a["before_state"]
    require(set(CHANGED_TABLES) <= set(s["inventory"]["tables"])
            and SEQUENCE in s["inventory"]["sequences"], "subscription_inventory_incomplete")
    require(all(s["tables"][t]["rows"] == 0 for t in CHANGED_TABLES), "subscription_seed_not_empty")
    seq = s["sequences"][SEQUENCE]
    require(type(seq["increment"]) is int and seq["increment"] == 1
            and type(seq["cache"]) is int and seq["cache"] == 1 and seq["cycle"] is False
            and type(seq["is_called"]) is bool, "subscription_sequence_noncanonical")
    last = integer(seq["last_value"], "subscription_sequence_invalid", minimum=1)
    maximum = integer(seq["max"], "subscription_sequence_invalid", minimum=1)
    require(last + int(seq["is_called"]) + len(m["records"]) <= maximum,
            "subscription_sequence_exhausted")


def _envelope(m):
    a = m["financial_attestation"]
    return {**a, "migration_id": m["migration_id"]}


def _quote(sql):
    tag = "$lmm_sub_" + F.sha(sql.encode())[:24] + "$"
    require(tag not in sql, "subscription_sql_delimiter_collision")
    prefix, sep, suffix = sql.rpartition("END $lmm_sub$;")
    require(bool(sep), "subscription_sql_template_invalid")
    return prefix.replace("DO $lmm_sub$", "DO " + tag, 1) + "END " + tag + ";" + suffix


def _start(m, readonly=False):
    # Reuse the session fence before taking the SERIALIZABLE snapshot.
    return F._start(_envelope(m), readonly=readonly)


def build_apply_sql(m):
    validate_material(m)
    e = _envelope(m)
    inv = e["before_state"]["inventory"]
    state = F.state_sql(inv, m["migration_id"])
    guard = F._guard(e)
    seq = e["before_state"]["sequences"][SEQUENCE]
    next_id = seq["last_value"] + int(seq["is_called"])
    changed = "ARRAY[" + ",".join(F.literal(t) for t in CHANGED_TABLES) + "]::text[]"
    sql = _start(m) + f"""
DO $lmm_sub$
DECLARE m jsonb := {F.literal(m)}::jsonb; p jsonb; previous core_migration.import_runs%ROWTYPE;
 before_state jsonb; after_state jsonb; sid bigint; ref text;
BEGIN
 {guard}
 LOCK TABLE {','.join(F.relation(t) for t in inv['tables'])} IN ACCESS EXCLUSIVE MODE;
 {guard}
 SELECT ({state}) INTO before_state;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->'financial_attestation'->>'source_archive_sha256'
   OR previous.mapping_sha256<>m->>'mapping_sha256' OR previous.manifest->'material' IS DISTINCT FROM m
   OR previous.manifest->>'scope' IS DISTINCT FROM {F.literal(SCOPE)}
   OR previous.manifest->'post_state' IS DISTINCT FROM before_state
  THEN RAISE EXCEPTION 'subscription_replay_conflict'; END IF;
 ELSE
  IF before_state IS DISTINCT FROM m->'financial_attestation'->'before_state'
   OR EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope'={F.literal(SCOPE)})
  THEN RAISE EXCEPTION 'subscription_target_state_changed'; END IF;
  IF NOT EXISTS(SELECT 1 FROM core_migration.import_runs r
   WHERE to_jsonb(r)=m->'financial_attestation'->'identity_import')
   OR (SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'personal_account_id',personal_account_id) ORDER BY id),'[]'::jsonb)
       FROM core_identity.users) IS DISTINCT FROM m->'financial_attestation'->'native_users'
  THEN RAISE EXCEPTION 'subscription_identity_binding_mismatch'; END IF;
  ALTER SEQUENCE {F.relation(SEQUENCE)} RESTART WITH {next_id};
  FOR p IN SELECT value FROM jsonb_array_elements(m->'records') LOOP
   IF NOT EXISTS(SELECT 1 FROM core_identity.users WHERE id=(p->>'source_user_id')::bigint
       AND personal_account_id=(p->>'account_id')::bigint)
   THEN RAISE EXCEPTION 'subscription_owner_mismatch'; END IF;
   ref := 'legacy-sub:' || (m->>'migration_id') || ':' || (p->>'source_subscription_id');
   INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,timezone,anchor,seconds,ends_at,
     limit_credits,unlimited,reset_basis,reset_limit_credits,reset_unlimited,models,groups,active)
   VALUES((p->>'account_id')::bigint,ref,p->>'period',p->>'timezone',(p->>'anchor')::bigint,
     (p->>'seconds')::bigint,(p->>'ends_at')::bigint,(p->>'limit_credits')::bigint,(p->>'unlimited')::boolean,
     p->>'reset_basis',(p->>'reset_limit_credits')::bigint,(p->>'reset_unlimited')::boolean,
     ARRAY(SELECT jsonb_array_elements_text(p->'models')),ARRAY(SELECT jsonb_array_elements_text(p->'groups')),
     (p->>'active')::boolean) RETURNING id INTO sid;
   INSERT INTO core_charging.subscription_opening_consumption(subscription_id,start_at,end_at,consumed_credits,import_reference)
   VALUES(sid,(p->>'opening_start')::bigint,(p->>'opening_end')::bigint,(p->>'opening_used')::bigint,ref);
  END LOOP;
  SET CONSTRAINTS ALL IMMEDIATE;
  IF (SELECT count(*) FROM core_charging.subscriptions)<>jsonb_array_length(m->'records')
   OR (SELECT count(*) FROM core_charging.subscription_opening_consumption)<>jsonb_array_length(m->'records')
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(m->'records') expected(value)
    LEFT JOIN core_charging.subscriptions s ON s.grant_reference=
      'legacy-sub:' || (m->>'migration_id') || ':' || (expected.value->>'source_subscription_id')
    LEFT JOIN core_charging.subscription_opening_consumption o ON o.subscription_id=s.id
    WHERE (to_jsonb(s)-'id'-'grant_reference') IS DISTINCT FROM
      (expected.value - ARRAY['source_subscription_id','source_user_id','source_row_sha256','plan_row_sha256',
                 'opening_start','opening_end','opening_used']::text[])
     OR o.start_at IS DISTINCT FROM (expected.value->>'opening_start')::bigint
     OR o.end_at IS DISTINCT FROM (expected.value->>'opening_end')::bigint
     OR o.consumed_credits IS DISTINCT FROM (expected.value->>'opening_used')::bigint
     OR o.import_reference IS DISTINCT FROM s.grant_reference)
  THEN RAISE EXCEPTION 'subscription_readback_failed'; END IF;
  SELECT ({state}) INTO after_state;
  IF (before_state->'tables') - {changed} IS DISTINCT FROM (after_state->'tables') - {changed}
   OR (before_state->'sequences') - {F.literal(SEQUENCE)} IS DISTINCT FROM (after_state->'sequences') - {F.literal(SEQUENCE)}
  THEN RAISE EXCEPTION 'subscription_unrelated_state_changed'; END IF;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->'financial_attestation'->>'source_archive_sha256',m->>'mapping_sha256',
    jsonb_build_object('scope',{F.literal(SCOPE)},'material',m,'post_state',after_state));
 END IF;
 {guard}
END $lmm_sub$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{F.literal(m['migration_id'])},
 'material_sha256',{F.literal(F.digest(m))});
"""
    return _quote(sql)


def verify_approval(m, sql, approval):
    validate_material(m)
    require(type(approval) is dict and set(approval) == {"format_version", "scope", "decision",
        "material_sha256", "sql_sha256", "source_review_sha256"}
        and type(approval["format_version"]) is int and approval["format_version"] == 1
        and approval["scope"] == SCOPE and approval["decision"] == "approve-offline-subscriptions"
        and approval["material_sha256"] == F.digest(m)
        and approval["sql_sha256"] == F.sha(sql.encode())
        and approval["source_review_sha256"] == F.digest(m["source_review"])
        and sql == build_apply_sql(m), "subscription_approval_mismatch")


def build_resolve_sql(m, original_application_name):
    validate_material(m)
    require(type(original_application_name) is str and re.fullmatch(r"lmm-subscription-[0-9a-f]{32}", original_application_name),
            "subscription_original_attempt_invalid")
    e = _envelope(m)
    state = F.state_sql(e["before_state"]["inventory"], m["migration_id"])
    sql = _start(m, readonly=True) + f"""
DO $lmm_sub$
DECLARE m jsonb := {F.literal(m)}::jsonb; previous core_migration.import_runs%ROWTYPE; actual jsonb;
BEGIN
 {F._guard(e)}
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name={F.literal(original_application_name)}
  AND datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid())
 THEN RAISE EXCEPTION 'subscription_original_attempt_active'; END IF;
 SELECT ({state}) INTO actual;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->'financial_attestation'->>'source_archive_sha256'
   OR previous.mapping_sha256<>m->>'mapping_sha256' OR previous.manifest->'material' IS DISTINCT FROM m
   OR previous.manifest->>'scope' IS DISTINCT FROM {F.literal(SCOPE)}
   OR previous.manifest->'post_state' IS DISTINCT FROM actual
  THEN RAISE EXCEPTION 'subscription_resolution_conflict'; END IF;
 ELSIF actual IS DISTINCT FROM m->'financial_attestation'->'before_state'
 THEN RAISE EXCEPTION 'subscription_absence_not_proven'; END IF;
END $lmm_sub$;
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs
 WHERE migration_id={F.literal(m['migration_id'])}) THEN 'COMMITTED' ELSE 'NOT_APPLIED' END,
 'material_sha256',{F.literal(F.digest(m))});
COMMIT;
"""
    return _quote(sql)
