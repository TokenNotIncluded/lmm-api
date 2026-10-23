"""Map frozen historical bounty rights to a review-only native escrow candidate.

No connections, SQL execution, approval creation, or filesystem writes occur here.
The pinned legacy_bounty_basis verifies parent-plan after images. Equal integers
and hash-bound unit evidence still require independent economic review; the
candidate deliberately lacks the SQL importer's review_proof_sha256 field.
"""

import copy
import re

import legacy_bounty_basis as BASIS

SCOPE = "legacy-escrow-candidate-only"
UNIT = "credit_500k_usd"
POLICY = "preserve-exact-parent-after-integers"
HASH = re.compile(r"[0-9a-f]{64}")
REVISION = re.compile(r"[0-9a-f]{40}")
UNIT_FIELDS = frozenset((
    "format_version", "scope", "source_archive_sha256", "mapping_sha256",
    "source_export_sha256", "identity_records_sha256", "parent_plan_sha256",
    "parent_audit_row_sha256", "unit", "credits_per_usd", "conversion_policy",
    "source_application_revision", "source_policy_evidence_sha256",
))


def _hash(F, value, code):
    F.require(type(value) is str and HASH.fullmatch(value) is not None, code)
    return value


def _uint(F, value, *, positive=False):
    return F.integer(value, "escrow_source_amount_or_time_invalid",
                     minimum=int(positive), maximum=F.JS_SAFE_MAX)


def _id(F, value):
    return F.integer(value, "escrow_identity_invalid", minimum=1)


def _identities(F, records, plan):
    F.require(type(records) is list and bool(records), "escrow_identity_records_missing")
    result, users, accounts = {}, set(), set()
    for row in records:
        F.require(type(row) is dict and set(row) == {
            "source_user_id", "target_user_id", "target_personal_account_id"}, "escrow_identity_shape_invalid")
        source, user, account = (_id(F, row[key]) for key in (
            "source_user_id", "target_user_id", "target_personal_account_id"))
        F.require(source not in result and user not in users and account not in accounts,
                  "escrow_identity_duplicate")
        result[source] = row
        users.add(user)
        accounts.add(account)
    F.require(F.digest({uid: row["target_personal_account_id"] for uid, row in result.items()})
              == plan.get("identity_map_sha256"), "escrow_identity_plan_mismatch")
    return result


def _bound_unit_evidence(F, evidence, *, plan, source_proof, identity_hash, binding):
    """Bind supplied evidence bytes semantically; do not attest their authority."""
    if evidence is None:
        return False
    F.require(type(evidence) is dict and set(evidence) == UNIT_FIELDS,
              "escrow_unit_evidence_shape_invalid")
    F.require(type(evidence["format_version"]) is int and evidence["format_version"] == 1
              and evidence["scope"] == "legacy-escrow-source-unit-evidence"
              and evidence["unit"] == UNIT and type(evidence["credits_per_usd"]) is int
              and evidence["credits_per_usd"] == 500000 and evidence["conversion_policy"] == POLICY,
              "escrow_unit_evidence_policy_invalid")
    expected = {"source_archive_sha256": plan["source_archive_sha256"],
        "mapping_sha256": plan["mapping_sha256"], "source_export_sha256": source_proof["source_export_sha256"],
        "identity_records_sha256": identity_hash, "parent_plan_sha256": binding["plan_sha256"],
        "parent_audit_row_sha256": binding["source_audit_row_sha256"]}
    for key, value in expected.items():
        F.require(_hash(F, evidence[key], "escrow_unit_evidence_hash_invalid") == value,
                  "escrow_unit_evidence_binding_mismatch")
    F.require(type(evidence["source_application_revision"]) is str
              and REVISION.fullmatch(evidence["source_application_revision"]) is not None,
              "escrow_source_application_revision_invalid")
    _hash(F, evidence["source_policy_evidence_sha256"], "escrow_unit_policy_evidence_missing")
    return True


# @mark migration.legacy-escrow-map role=implementation
# @mark migration.bounty-basis role=implementation
def build(F, *, tables, identity_records, plan, migration_id, observed_at,
          source_proof, unit_evidence=None):
    """Return a candidate and source blockers, never an executable import batch.

    Caller must verify source_export bytes/proof and identity-map provenance.
    Unit evidence is a separate source artifact for the economic reviewer. It
    binds archive/export/mapping/identity/parent/code evidence and cannot replace
    review approval. Unknown unit evidence produces no monetary batch at all.
    """
    F.verify_plan_mapping(plan)
    F.require(type(migration_id) is str and F.IDENTITY.MIGRATION_ID.fullmatch(migration_id) is not None,
              "escrow_migration_id_invalid")
    observed_at = F.integer(observed_at, "escrow_observation_invalid", minimum=1, maximum=253402300799)
    archive = _hash(F, plan.get("source_archive_sha256"), "escrow_archive_hash_invalid")
    F.require(type(source_proof) is dict and source_proof.get("source_archive_sha256") == archive,
              "escrow_source_proof_archive_mismatch")
    export_hash = _hash(F, source_proof.get("source_export_sha256"), "escrow_export_hash_invalid")
    identities = _identities(F, identity_records, plan)
    identity_hash = F.digest(sorted(identity_records, key=lambda row: row["source_user_id"]))
    account_map = {uid: row["target_personal_account_id"] for uid, row in identities.items()}
    for table in (*BASIS.BOUNTY_TABLES, "wallet_credit_rebases"):
        F.require(table in tables and table in plan["source_evidence"], "escrow_source_table_coverage_incomplete")
        count = source_proof.get("selected_table_counts", {}).get(table)
        F.require(type(count) is int and count == len(tables[table]) == plan["source_evidence"][table]["row_count"],
                  "escrow_source_proof_count_mismatch")
    lineage = BASIS.build(F, tables=tables, identities=account_map, plan=plan)
    blockers = [code for code in lineage["blockers"]
                if code != "bounty_economic_approval_and_dump_epoch_unit_authority_missing"]
    bindings = lineage["parent_plan_bindings"]
    parent = bindings[0] if len(bindings) == 1 else None
    lineage_ok = lineage["lineage_status"] == "matches-parent-after-with-unverified-authority"
    if not lineage_ok:
        blockers.append("escrow_exact_parent_after_lineage_required")
    if unit_evidence is not None:
        F.require(parent is not None, "escrow_unit_evidence_requires_single_parent")
    unit_bound = (_bound_unit_evidence(F, unit_evidence, plan=plan, source_proof=source_proof,
                                     identity_hash=identity_hash, binding=parent)
                  if parent is not None else False)
    if not unit_bound:
        blockers.append("escrow_source_unit_evidence_required")
    if parent is not None and observed_at < parent["snapshot_at"]:
        blockers.append("escrow_observation_precedes_parent_snapshot")
    lineage_rows = {(row["table"], row["source_primary_key"]["id"]): row for row in lineage["records"]}
    project_rows, claim_rows, dispute_rows = (tables[name] for name in BASIS.BOUNTY_TABLES)
    funded = {}
    for row in project_rows:
        quota = _uint(F, row.get("escrow_quota"))
        if quota:
            funded[_id(F, row["id"])] = row
    if not 1 <= len(funded) <= 128:
        blockers.append("escrow_positive_project_batch_size_unsupported")
    proposals, paid_skipped, accepted_count, guard_count, claim_count, dispute_count = [], 0, 0, 0, 0, 0
    seen_claims, seen_disputes = set(), set()
    for pid, project in sorted(funded.items()):
        owner_source = _id(F, project["owner_user_id"])
        F.require(owner_source in identities, "escrow_original_payer_unmapped")
        owner = identities[owner_source]
        if project["status"] not in ("published", "paused"):
            blockers.append("escrow_nonactive_funded_project_unsupported")
        parent_row = lineage_rows.get((BASIS.BOUNTY_TABLES[0], pid))
        if parent_row is None or parent_row["state"] not in ("after", "before_and_after"):
            blockers.append("escrow_project_parent_after_missing")
        claims, disputes = [], []
        for source in sorted(claim_rows, key=lambda row: row["id"]):
            if _id(F, source["project_id"]) != pid:
                continue
            paid_at = _uint(F, source["paid_at"])
            if paid_at:
                paid_skipped += 1
                continue
            if source["status"] not in ("accepted", "submitted", "rejected"):
                if any(d["challenge_id"] == source["id"] and d["status"] == "open" for d in dispute_rows):
                    blockers.append("escrow_open_dispute_claim_state_unsupported")
                continue
            cid = _id(F, source["id"])
            F.require(cid not in seen_claims, "escrow_duplicate_source_claim")
            seen_claims.add(cid)
            participant_source = _id(F, source["participant_user_id"])
            F.require(participant_source in identities, "escrow_claim_participant_unmapped")
            participant = identities[participant_source]
            F.require(participant_source != owner_source, "escrow_claim_payer_is_recipient")
            basis = lineage_rows.get((BASIS.BOUNTY_TABLES[1], cid))
            right = basis.get("rights_status") if basis else None
            if basis is None or basis["state"] not in ("after", "before_and_after"):
                blockers.append("escrow_claim_parent_after_missing")
            if right not in ("active_future_reward", "historical_rejection_guard_only"):
                blockers.append("escrow_claim_right_basis_missing")
            if source["status"] in ("accepted", "submitted") and right != "active_future_reward":
                blockers.append("escrow_active_claim_right_basis_invalid")
            rejected_at = _uint(F, source["rejected_at"])
            if rejected_at > observed_at or (source["status"] == "rejected" and rejected_at == 0):
                blockers.append("escrow_claim_rejection_time_invalid")
            reward = _uint(F, source["reward_quota"])
            if right == "active_future_reward" and reward == 0:
                blockers.append("escrow_active_claim_zero_reward_unsupported")
            claims.append({"source_challenge_id": cid, "participant_user_id": participant["target_user_id"],
                "recipient_account_id": participant["target_personal_account_id"], "source_row_sha256": F.digest(source),
                "rights_status": right, "source_reward_quota": reward,
                "payout_units": reward if unit_bound and right == "active_future_reward" else None,
                "status": source["status"], "rejected_at": rejected_at, "paid_at": paid_at})
            accepted_count += source["status"] == "accepted"
            guard_count += right == "historical_rejection_guard_only"
        claim_index = {row["source_challenge_id"]: row for row in claims}
        for source in sorted(dispute_rows, key=lambda row: row["id"]):
            if _id(F, source["project_id"]) != pid:
                continue
            did, cid = _id(F, source["id"]), _id(F, source["challenge_id"])
            F.require(did not in seen_disputes, "escrow_duplicate_source_dispute")
            seen_disputes.add(did)
            if cid not in claim_index:
                blockers.append("escrow_dispute_unpaid_claim_binding_missing")
            if source["status"] not in ("open", "resolved_paid", "resolved_denied"):
                blockers.append("escrow_dispute_status_unsupported")
            if source["status"] == "resolved_paid":
                blockers.append("escrow_resolved_paid_dispute_not_new_payable_right")
            basis = lineage_rows.get((BASIS.BOUNTY_TABLES[2], did))
            if basis is None or basis["state"] not in ("after", "before_and_after"):
                blockers.append("escrow_dispute_parent_after_missing")
            disputes.append({"source_dispute_id": did, "source_challenge_id": cid,
                "source_row_sha256": F.digest(source), "status": source["status"],
                **{field: _uint(F, source[field]) for field in (
                    "reward_quota_snapshot", "project_escrow_quota_snapshot", "tip_quota_snapshot")}})
        owed = 0
        for claim in claims:
            cases = [d for d in disputes if d["source_challenge_id"] == claim["source_challenge_id"]]
            appeal = any(d["status"] == "open" for d in cases) or (
                claim["rejected_at"] > observed_at - 604800
                and not any(d["status"] in ("resolved_paid", "resolved_denied") for d in cases))
            if claim["rights_status"] == "historical_rejection_guard_only" and appeal:
                blockers.append("escrow_active_appeal_requires_current_unit_basis")
            if claim["rights_status"] == "active_future_reward" and (claim["status"] in ("accepted", "submitted") or appeal):
                owed += claim["source_reward_quota"]
        if owed > project["escrow_quota"]:
            blockers.append("escrow_cannot_cover_unpaid_commitments")
        if len(claims) > 2048 or len(disputes) > 2048:
            blockers.append("escrow_project_rights_size_unsupported")
        candidate = {"source_project_id": pid, "owner_user_id": owner["target_user_id"],
            "payer_account_id": owner["target_personal_account_id"], "source_row_sha256": F.digest(project),
            "source_parent_plan_sha256": parent["plan_sha256"] if parent is not None else None,
            "escrow_units": project["escrow_quota"] if unit_bound else None,
            "status": project["status"], "claims": claims, "disputes": disputes}
        proposals.append({"source_project_id": pid, "source_owner_user_id": owner_source,
            "source_escrow_quota": project["escrow_quota"], "source_unit": "unproven-until-independent-economic-review",
            "source_integer_owed": owed, "candidate": candidate})
        claim_count += len(claims)
        dispute_count += len(disputes)
    for source in claim_rows:
        if source.get("paid_at") == 0 and source.get("status") in ("accepted", "submitted") and source.get("project_id") not in funded:
            blockers.append("escrow_continuing_claim_outside_funded_projects")
    blockers = sorted(set(blockers))
    batch = None
    if not blockers:
        batch = {"migration_id": migration_id, "source_archive_sha256": archive,
            "mapping_sha256": plan["mapping_sha256"], "unit": UNIT, "observed_at": observed_at,
            "projects": [copy.deepcopy(row["candidate"]) for row in proposals]}
        if len(F.canonical(batch).encode("utf-8")) + 100 > 1048576:
            blockers.append("escrow_candidate_batch_size_unsupported")
            batch = None
    flags = {"economic_approval_verified": False, "approval_created": False, "import_payload_created": False,
        "target_queries": 0, "target_writes": 0, "source_queries": 0,
        "full_database_rights_cleared": False, "source_unit_authority_verified": False,
        "inflight_reservations_verified": False}
    summary = {"format_version": 1, "scope": SCOPE,
        "status": "blocked-for-source-evidence" if blockers else "candidate-requires-economic-review",
        "project_count": len(proposals), "claim_count": claim_count,
        "accepted_unpaid_claim_count": accepted_count, "historical_rejection_guard_count": guard_count,
        "dispute_count": dispute_count, "skipped_paid_claim_count": paid_skipped,
        "blockers": blockers, **flags}
    result = {**summary, "migration_id": migration_id, "source_archive_sha256": archive,
        "source_export_sha256": export_hash, "mapping_sha256": plan["mapping_sha256"],
        "identity_map_sha256": plan["identity_map_sha256"], "identity_records_sha256": identity_hash,
        "observed_at": observed_at, "observation_basis": "caller-supplied-capture-start-not-cutover-or-drain-proof",
        "source_unit_evidence_bound": unit_bound,
        "unit_evidence_sha256": F.digest(unit_evidence) if unit_evidence is not None else None,
        "source_parent_plan_bindings": copy.deepcopy(bindings), "lineage": lineage,
        "wallet_debit_policy": "restore-clearing-to-reserved-without-wallet-debit",
        "current_amounts_rescaled": False, "dispute_snapshots_rescaled": False,
        "projects": proposals, "batch_candidate": batch}
    result["candidate_sha256"] = F.digest(result)
    result["public_summary"] = summary
    return result
