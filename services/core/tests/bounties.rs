//! Developer PG checks; direct synthetic confirmation approval is not browser acceptance.
use lmm_core::{
    billing::{
        Credits,
        ledger::{Action, Ledger, Outcome, Request},
    },
    bounties::{self, BountyStore, Command, Error},
    core_schema,
    identity::IdentityStore,
};
use sqlx::PgPool;
async fn fixture(pool: &PgPool) -> (BountyStore, String, String, String, i64, i64) {
    core_schema::init(pool).await.unwrap();
    let exists: bool = sqlx::query_scalar("SELECT to_regnamespace('core_bounties') IS NOT NULL")
        .fetch_one(pool)
        .await
        .unwrap();
    if !exists {
        sqlx::raw_sql(bounties::SCHEMA).execute(pool).await.unwrap();
    }
    let ids = IdentityStore::from_pool(pool.clone());
    let owner = ids.bootstrap_user(7, 1).await.unwrap().secret;
    let recipient = ids.bootstrap_user(9, 1).await.unwrap().secret;
    let admin = ids.bootstrap_user(11, 6).await.unwrap().secret;
    let payer = ids.authorize(&owner).await.unwrap().owner_account_id;
    let receiver = ids.authorize(&recipient).await.unwrap().owner_account_id;
    let fees = ids.authorize(&admin).await.unwrap().owner_account_id;
    sqlx::query("INSERT INTO core_bounties.policy(id,fee_bps,fee_account_id) VALUES(TRUE,100,$1)")
        .bind(fees)
        .execute(pool)
        .await
        .unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(payer).await.unwrap();
    assert!(matches!(
        ledger
            .execute(&Request {
                scope: "bounty-fixture".into(),
                operation_key: "synthetic-credit".into(),
                actor_user_id: Some(7),
                reason: "Synthetic bounty PG fixture".into(),
                action: Action::Credit {
                    account_id: payer,
                    amount_units: Credits::try_from(1000000).unwrap(),
                    payment_reference: "synthetic-bounty-credit".into()
                }
            })
            .await
            .unwrap(),
        Outcome::Posted { .. }
    ));
    (
        BountyStore::from_pool(pool.clone()),
        owner,
        recipient,
        admin,
        payer,
        receiver,
    )
}
fn cmd(op: &str, action: &str) -> Command {
    Command {
        operation_id: op.into(),
        action: action.into(),
        project_id: "synthetic-project".into(),
        challenge_id: "synthetic-claim".into(),
        ..Default::default()
    }
}
async fn approved(pool: &PgPool, s: &BountyStore, secret: &str, c: &mut Command) {
    let p = s.preview(secret, c).await.unwrap();
    c.confirmation = p.confirmation;
    sqlx::query("UPDATE core_bounties.confirmations SET approved=TRUE WHERE id=$1")
        .bind(&c.confirmation)
        .execute(pool)
        .await
        .unwrap();
}
async fn publish(pool: &PgPool, s: &BountyStore, owner: &str) {
    let mut c = cmd("publish", "publish");
    c.reward_credit = 10001;
    c.reward_slots = 2;
    c.repository_url = "https://github.com/synthetic/bounty".into();
    approved(pool, s, owner, &mut c).await;
    let r = s.execute(owner, &c).await.unwrap();
    assert_eq!(
        (r.credit, r.reward_credit, r.escrow_credit),
        (20002, 9900, 19800)
    );
    assert_eq!(r.journal_ids.len(), 3);
}
#[test]
fn exact_per_slot_fee_and_overflow() {
    assert_eq!(
        bounties::publication(10001, 2, 100).unwrap(),
        (20002, 202, 9900)
    );
    assert!(bounties::publication(i64::MAX, 2, 100).is_err());
    assert!(bounties::publication(1, 1, 100).is_err());
}
#[sqlx::test(migrations = false)]
async fn frozen_recipient_exact_payout_replay(pool: PgPool) {
    let (s, owner, recipient, _, payer, receiver) = fixture(&pool).await;
    publish(&pool, &s, &owner).await;
    let mut c = cmd("accept", "accept");
    c.github_handle = "synthetic".into();
    approved(&pool, &s, &recipient, &mut c).await;
    s.execute(&recipient, &c).await.unwrap();
    let mut c = cmd("submit", "submit");
    c.pull_request_url = "https://github.com/synthetic/bounty/pull/1".into();
    approved(&pool, &s, &recipient, &mut c).await;
    s.execute(&recipient, &c).await.unwrap();
    let mut c = cmd("approve", "approve");
    approved(&pool, &s, &owner, &mut c).await;
    let first = s.execute(&owner, &c).await.unwrap();
    let replay = s.execute(&owner, &c).await.unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.credit, 9900);
    let ledger = Ledger::from_pool(pool.clone());
    assert_eq!(
        ledger.balance(receiver).await.unwrap().available_units,
        9900
    );
    assert_eq!(ledger.balance(payer).await.unwrap().reserved_units, 9900);
    c.credit = 1;
    assert!(matches!(s.execute(&owner, &c).await, Err(Error::Conflict)));
    assert_eq!(first.remaining_credit, None);
    let mut tip = cmd("tip-exact-balance", "tip");
    tip.credit = 123;
    approved(&pool, &s, &owner, &mut tip).await;
    let receipt = s.execute(&owner, &tip).await.unwrap();
    assert_eq!(
        receipt.remaining_credit,
        Some(ledger.balance(payer).await.unwrap().available_units)
    );
    assert_eq!(s.query(&owner, &tip.operation_id).await.unwrap(), receipt);
}
#[sqlx::test(migrations = false)]
async fn unconfirmed_and_foreign_sessions_cannot_move_money(pool: PgPool) {
    let (s, owner, recipient, _, payer, _) = fixture(&pool).await;
    let mut c = cmd("pending", "publish");
    c.reward_credit = 10000;
    c.reward_slots = 1;
    c.repository_url = "https://github.com/synthetic/bounty".into();
    c.confirmation = s.preview(&owner, &c).await.unwrap().confirmation;
    assert!(matches!(s.execute(&owner, &c).await, Err(Error::Forbidden)));
    sqlx::query("UPDATE core_bounties.confirmations SET approved=TRUE WHERE id=$1")
        .bind(&c.confirmation)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        s.execute(&recipient, &c).await,
        Err(Error::Forbidden)
    ));
    assert_eq!(
        Ledger::from_pool(pool)
            .balance(payer)
            .await
            .unwrap()
            .reserved_units,
        0
    );
}
#[sqlx::test(migrations = false)]
async fn submitted_and_appealable_claims_protect_original_hold(pool: PgPool) {
    let (s, owner, recipient, _, payer, _) = fixture(&pool).await;
    publish(&pool, &s, &owner).await;
    let mut c = cmd("accept", "accept");
    c.github_handle = "synthetic".into();
    approved(&pool, &s, &recipient, &mut c).await;
    s.execute(&recipient, &c).await.unwrap();
    let mut c = cmd("close-pending", "close");
    approved(&pool, &s, &owner, &mut c).await;
    assert!(matches!(s.execute(&owner, &c).await, Err(Error::Conflict)));
    let mut c = cmd("submit", "submit");
    c.issue_url = "https://github.com/synthetic/bounty/issues/1".into();
    approved(&pool, &s, &recipient, &mut c).await;
    s.execute(&recipient, &c).await.unwrap();
    let mut c = cmd("reject", "reject");
    approved(&pool, &s, &owner, &mut c).await;
    s.execute(&owner, &c).await.unwrap();
    let mut c = cmd("close-appeal", "close");
    approved(&pool, &s, &owner, &mut c).await;
    assert!(matches!(s.execute(&owner, &c).await, Err(Error::Conflict)));
    assert_eq!(
        Ledger::from_pool(pool)
            .balance(payer)
            .await
            .unwrap()
            .reserved_units,
        19800
    );
}

async fn imported(pool: &PgPool, id: i64) {
    sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES($1,decode(repeat('a',64),'hex'),decode(repeat('b',64),'hex'),decode(repeat('c',64),'hex'),decode(repeat('d',64),'hex'),'bounty-synthetic-policy',decode(repeat('e',64),'hex'),(SELECT oid FROM pg_database WHERE datname=current_database()),decode(repeat('f',64),'hex'),decode(repeat('1',64),'hex'),true,false,true,true,decode(repeat('2',64),'hex'),decode(repeat('3',64),'hex'),'superadmin',false,1)").bind(id).execute(pool).await.unwrap();
}
async fn financial_snapshot(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('projects',(SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY id),'[]') FROM core_bounties.projects x),'journals',(SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY id),'[]') FROM core_billing.ledger_journals x),'entries',(SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY journal_id,ledger_account_id),'[]') FROM core_billing.ledger_entries x),'balances',(SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY ledger_account_id),'[]') FROM core_billing.balance_state x),'operations',(SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY id),'[]') FROM core_bounties.operations x))").fetch_one(pool).await.unwrap()
}
#[sqlx::test(migrations = false)]
async fn imported_admin_policy_gate_preserves_escrow_and_complete_admin_can_resolve(pool: PgPool) {
    use lmm_core::identity::{canonical_policy_payload, complete_policy_snapshot, policy_sha256};
    let (s, owner, recipient, admin, _, receiver) = fixture(&pool).await;
    publish(&pool, &s, &owner).await;
    for (action, token) in [
        ("accept", &recipient),
        ("submit", &recipient),
        ("reject", &owner),
        ("open_dispute", &recipient),
    ] {
        let mut c = cmd(action, action);
        c.github_handle = "synthetic".into();
        c.issue_url = "https://github.com/synthetic/bounty/issues/1".into();
        c.dispute_id = "case-one".into();
        c.reason = "requirements_met_but_rejected".into();
        c.statement = "Synthetic contributor dispute evidence for protected escrow".into();
        approved(&pool, &s, token, &mut c).await;
        s.execute(token, &c).await.unwrap();
    }
    let ids = IdentityStore::from_pool(pool.clone());
    for (id, kind) in [(13, "missing"), (15, "incomplete"), (17, "bad_hash")] {
        let secret = ids.bootstrap_user(id, 6).await.unwrap().secret;
        imported(&pool, id).await;
        if kind == "incomplete" {
            // Canonical storage forbids verified_complete=false. Persist a valid
            // hash/payload header, then only eight of the required nine rows.
            let entries: Vec<(String, String)> = [
                "ChannelRead",
                "ChannelOperate",
                "ChannelWrite",
                "ChannelSensitiveWrite",
                "ChannelSecretView",
                "AcquisitionRead",
                "AcquisitionWrite",
                "AcquisitionDetails",
                "AcquisitionExport",
            ]
            .into_iter()
            .map(|v| (v.into(), "deny".into()))
            .collect();
            let snapshot = complete_policy_snapshot(id, 1, &entries).unwrap();
            let payload = canonical_policy_payload(&snapshot).unwrap();
            let digest = policy_sha256(&snapshot).unwrap();
            sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES($1,1,'core.permissions.v1',$2,$3,true)").bind(id).bind(digest.to_vec()).bind(payload).execute(&pool).await.unwrap();
            for (action, effect) in entries.into_iter().take(8) {
                sqlx::query("INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES($1,$2,$3)").bind(id).bind(action).bind(effect).execute(&pool).await.unwrap();
            }
        } else if kind == "bad_hash" {
            sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES($1,1,'core.permissions.v1',decode(repeat('0',64),'hex'),convert_to('{}','UTF8'),true)").bind(id).execute(&pool).await.unwrap();
        }
        {
            use lmm_core::identity::{PolicyLoadError, load_complete_policy};
            let mut connection = pool.acquire().await.unwrap();
            let error = load_complete_policy(&mut connection, id)
                .await
                .err()
                .expect("fixture must be rejected before business mutation");
            assert!(matches!(
                (kind, error),
                ("missing", PolicyLoadError::Missing)
                    | ("incomplete", PolicyLoadError::Incomplete)
                    | ("bad_hash", PolicyLoadError::HashMismatch)
            ));
        }
        assert!(!s.session_authorize(&secret).await.unwrap().admin);
        assert!(matches!(s.authorize(&secret).await, Err(Error::Forbidden)));
        let mut c = cmd(&format!("resolve-{kind}"), "resolve_dispute");
        c.dispute_id = "case-one".into();
        c.resolve_action = "pay".into();
        c.resolution = "Synthetic administrator arbitration payout decision".into();
        approved(&pool, &s, &secret, &mut c).await;
        let before = financial_snapshot(&pool).await;
        assert!(matches!(
            s.execute(&secret, &c).await,
            Err(Error::Forbidden)
        ));
        assert_eq!(
            before,
            financial_snapshot(&pool).await,
            "{kind} changed funds or holds"
        );
    }
    imported(&pool, 11).await;
    let entries: Vec<(String, String)> = [
        "ChannelRead",
        "ChannelOperate",
        "ChannelWrite",
        "ChannelSensitiveWrite",
        "ChannelSecretView",
        "AcquisitionRead",
        "AcquisitionWrite",
        "AcquisitionDetails",
        "AcquisitionExport",
    ]
    .into_iter()
    .map(|v| (v.into(), "deny".into()))
    .collect();
    let snapshot = complete_policy_snapshot(11, 7, &entries).unwrap();
    let payload = canonical_policy_payload(&snapshot).unwrap();
    let hash = policy_sha256(&snapshot).unwrap();
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES(11,7,'core.permissions.v1',$1,$2,true)").bind(hash.to_vec()).bind(payload).execute(&pool).await.unwrap();
    for (action, effect) in entries {
        sqlx::query("INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES(11,$1,$2)").bind(action).bind(effect).execute(&pool).await.unwrap();
    }
    assert!(s.authorize(&admin).await.unwrap().admin);
    let mut c = cmd("resolve-complete", "resolve_dispute");
    c.dispute_id = "case-one".into();
    c.resolve_action = "pay".into();
    c.resolution = "Synthetic complete administrator policy payout decision".into();
    approved(&pool, &s, &admin, &mut c).await;
    assert_eq!(s.execute(&admin, &c).await.unwrap().credit, 9900);
    assert_eq!(
        Ledger::from_pool(pool)
            .balance(receiver)
            .await
            .unwrap()
            .available_units,
        9900
    );
}
async fn marked(pool: &PgPool, id: i64, on: bool) {
    sqlx::query("INSERT INTO core_abuse.user_marks(user_id,marked,policy_id,revision) VALUES($1,$2,CASE WHEN $2 THEN 'synthetic-bounty-mark' ELSE NULL END,1) ON CONFLICT(user_id) DO UPDATE SET marked=excluded.marked,policy_id=excluded.policy_id,revision=core_abuse.user_marks.revision+1").bind(id).bind(on).execute(pool).await.unwrap();
}
#[sqlx::test(migrations = false)]
async fn marked_new_actions_deny_without_blocking_original_hold_return(pool: PgPool) {
    let (s, owner, recipient, _, payer, _) = fixture(&pool).await;
    let mut c = cmd("marked-publish", "publish");
    c.reward_credit = 10001;
    c.reward_slots = 2;
    c.repository_url = "https://github.com/synthetic/bounty".into();
    approved(&pool, &s, &owner, &mut c).await;
    marked(&pool, 7, true).await;
    let before = financial_snapshot(&pool).await;
    assert!(matches!(s.execute(&owner, &c).await, Err(Error::Forbidden)));
    assert_eq!(before, financial_snapshot(&pool).await);
    marked(&pool, 7, false).await;
    publish(&pool, &s, &owner).await;
    let mut c = cmd("marked-accept", "accept");
    c.github_handle = "synthetic".into();
    approved(&pool, &s, &recipient, &mut c).await;
    marked(&pool, 9, true).await;
    let before = financial_snapshot(&pool).await;
    assert!(matches!(
        s.execute(&recipient, &c).await,
        Err(Error::Forbidden)
    ));
    assert_eq!(before, financial_snapshot(&pool).await);
    marked(&pool, 9, false).await;
    s.execute(&recipient, &c).await.unwrap();
    let mut c = cmd("submit", "submit");
    c.issue_url = "https://github.com/synthetic/bounty/issues/1".into();
    approved(&pool, &s, &recipient, &mut c).await;
    s.execute(&recipient, &c).await.unwrap();
    let mut c = cmd("marked-approve", "approve");
    approved(&pool, &s, &owner, &mut c).await;
    marked(&pool, 7, true).await;
    let before = financial_snapshot(&pool).await;
    assert!(matches!(s.execute(&owner, &c).await, Err(Error::Forbidden)));
    assert_eq!(before, financial_snapshot(&pool).await);
    marked(&pool, 7, false).await;
    s.execute(&owner, &c).await.unwrap();
    let mut c = cmd("marked-tip", "tip");
    c.credit = 10;
    approved(&pool, &s, &owner, &mut c).await;
    marked(&pool, 7, true).await;
    let before = financial_snapshot(&pool).await;
    assert!(matches!(s.execute(&owner, &c).await, Err(Error::Forbidden)));
    assert_eq!(before, financial_snapshot(&pool).await);
    let mut c = cmd("marked-close", "close");
    approved(&pool, &s, &owner, &mut c).await;
    assert_eq!(s.execute(&owner, &c).await.unwrap().credit, 9900);
    assert_eq!(
        Ledger::from_pool(pool)
            .balance(payer)
            .await
            .unwrap()
            .reserved_units,
        0
    );
}
#[sqlx::test(migrations = false)]
async fn query_recovers_only_original_actor_persisted_receipt_and_never_confirms_or_executes(
    pool: PgPool,
) {
    let (s, owner, recipient, _, _, _) = fixture(&pool).await;
    let mut c = cmd("query-original", "publish");
    c.reward_credit = 10001;
    c.reward_slots = 2;
    c.repository_url = "https://github.com/synthetic/bounty".into();
    let p = s.preview(&owner, &c).await.unwrap();
    c.confirmation = p.confirmation.clone();
    let before = financial_snapshot(&pool).await;
    let confirmations: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM core_bounties.confirmations c",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let pending = s.query(&owner, &c.operation_id).await.unwrap();
    assert_eq!(pending.state, "awaiting_confirmation");
    assert_eq!(pending.confirmation, p.confirmation);
    assert_eq!(pending.expires, p.expires);
    assert_eq!(before, financial_snapshot(&pool).await);
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM core_bounties.confirmations c",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(confirmations, after);
    assert!(matches!(
        s.query(&recipient, &c.operation_id).await,
        Err(Error::Missing)
    ));
    sqlx::query("UPDATE core_bounties.confirmations SET approved=TRUE WHERE id=$1")
        .bind(&c.confirmation)
        .execute(&pool)
        .await
        .unwrap();
    let before = financial_snapshot(&pool).await;
    assert_eq!(
        s.query(&owner, &c.operation_id).await.unwrap().state,
        "confirmed"
    );
    assert_eq!(before, financial_snapshot(&pool).await);
    let original = s.execute(&owner, &c).await.unwrap();
    let before = financial_snapshot(&pool).await;
    assert_eq!(original, s.query(&owner, &c.operation_id).await.unwrap());
    assert_eq!(before, financial_snapshot(&pool).await);
    assert!(matches!(
        s.query(&recipient, &c.operation_id).await,
        Err(Error::Missing)
    ));
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=7")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        s.query(&owner, &c.operation_id).await,
        Err(Error::Forbidden)
    ));
}
#[sqlx::test(migrations = false)]
async fn query_rejects_different_pending_payloads_for_same_original_key(pool: PgPool) {
    let (s, owner, _, _, _, _) = fixture(&pool).await;
    let mut c = cmd("query-conflict", "publish");
    c.reward_credit = 10001;
    c.reward_slots = 2;
    c.repository_url = "https://github.com/synthetic/bounty".into();
    s.preview(&owner, &c).await.unwrap();
    c.reward_credit = 20001;
    s.preview(&owner, &c).await.unwrap();
    let before = financial_snapshot(&pool).await;
    assert!(matches!(
        s.query(&owner, &c.operation_id).await,
        Err(Error::Conflict)
    ));
    assert_eq!(before, financial_snapshot(&pool).await);
}
#[sqlx::test(migrations = false)]
async fn projection_recovery_requires_all_original_arguments_without_writing(pool: PgPool) {
    let (s, owner, _, _, _, _) = fixture(&pool).await;
    let mut c = cmd("projection-original", "publish");
    c.reward_credit = 10001;
    c.reward_slots = 2;
    c.repository_url = "https://github.com/synthetic/bounty".into();
    c.submission_note = "Frozen original evidence".into();
    c.rating_score = 3;
    c.rating_comment = "Frozen original rating".into();
    approved(&pool, &s, &owner, &mut c).await;
    let before = financial_snapshot(&pool).await;
    assert_eq!(
        s.query_for_command(&owner, &c).await.unwrap().state,
        "confirmed"
    );
    for field in ["evidence", "rating", "project", "key"] {
        let mut altered = c.clone();
        match field {
            "evidence" => altered.submission_note = "Changed evidence".into(),
            "rating" => altered.rating_score = 5,
            "project" => altered.project_id = "other-project".into(),
            _ => altered.operation_id = "other-key".into(),
        };
        assert!(matches!(
            s.query_for_command(&owner, &altered).await,
            Err(Error::Conflict | Error::Missing)
        ));
    }
    assert_eq!(before, financial_snapshot(&pool).await);
    let original = s.execute(&owner, &c).await.unwrap();
    assert_eq!(original, s.query_for_command(&owner, &c).await.unwrap());
    let before = financial_snapshot(&pool).await;
    for field in ["evidence", "rating", "amount"] {
        let mut altered = c.clone();
        match field {
            "evidence" => altered.submission_note = "Changed after commit".into(),
            "rating" => altered.rating_comment = "Changed rating".into(),
            _ => altered.reward_credit += 1,
        };
        assert!(matches!(
            s.query_for_command(&owner, &altered).await,
            Err(Error::Conflict)
        ));
    }
    assert_eq!(before, financial_snapshot(&pool).await);
}
#[sqlx::test(migrations = false)]
async fn original_receipts_keep_project_revision_and_first_publish_time(pool: PgPool) {
    let (s, owner, _, _, _, _) = fixture(&pool).await;
    let mut publish = cmd("revision-publish", "publish");
    publish.reward_credit = 10001;
    publish.reward_slots = 2;
    publish.repository_url = "https://github.com/synthetic/bounty".into();
    approved(&pool, &s, &owner, &mut publish).await;
    let pending = s.query_for_command(&owner, &publish).await.unwrap();
    assert_eq!(pending.project_revision, 0);
    assert_eq!(pending.published_at, 0);
    let first = s.execute(&owner, &publish).await.unwrap();
    assert_eq!(first.project_revision, 1);
    assert!(first.published_at > 0);
    assert_eq!(first.fee_basis_points, Some(100));
    sqlx::query("UPDATE core_bounties.policy SET fee_bps=321 WHERE id")
        .execute(&pool)
        .await
        .unwrap();
    let mut pause = cmd("revision-pause", "pause");
    approved(&pool, &s, &owner, &mut pause).await;
    let newer = s.execute(&owner, &pause).await.unwrap();
    assert_eq!(newer.project_revision, 2);
    assert_eq!(newer.fee_basis_points, Some(100));
    assert_eq!(newer.published_at, first.published_at);
    assert_eq!(s.execute(&owner, &publish).await.unwrap(), first);
    assert_eq!(s.query_for_command(&owner, &publish).await.unwrap(), first);
    let body: serde_json::Value =
        sqlx::query_scalar("SELECT body FROM core_bounties.projects WHERE id=$1")
            .bind(&publish.project_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(body["revision"], 2);
    assert_eq!(body["status"], "paused");
    assert_eq!(body["published_at"], first.published_at);
    for body in [
        serde_json::json!({"revision":2}),
        serde_json::json!({"revision":4}),
        serde_json::json!({"revision":3,"published_at":first.published_at+1}),
    ] {
        assert!(
            sqlx::query("UPDATE core_bounties.projects SET body=body||$2 WHERE id=$1")
                .bind(&publish.project_id)
                .bind(body)
                .execute(&pool)
                .await
                .is_err()
        );
    }
    let after: serde_json::Value =
        sqlx::query_scalar("SELECT body FROM core_bounties.projects WHERE id=$1")
            .bind(&publish.project_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, body);
    let mut resume = cmd("revision-resume", "resume");
    approved(&pool, &s, &owner, &mut resume).await;
    let resumed = s.execute(&owner, &resume).await.unwrap();
    assert_eq!(resumed.project_revision, 3);
    assert_eq!(resumed.published_at, first.published_at);
}
// Isolated owner fixture simulates successful fresh primary authentication after
// the real grant revocation trigger invalidates previous Session generations.
async fn synthetic_reauthenticated_session(pool: &PgPool, user: i64) -> String {
    use sha2::{Digest, Sha256};
    let secret = format!("lmms_synthetic_bounty_reauthenticated_{user}_session");
    let id:i64=sqlx::query_scalar("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour' FROM core_identity.users WHERE id=$2 RETURNING id").bind(Sha256::digest(secret.as_bytes()).to_vec()).bind(user).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,factor_revision,factor_level,method,authenticated_at) SELECT $1,id,auth_version,0,1,'primary',clock_timestamp() FROM core_identity.users WHERE id=$2").bind(id).bind(user).execute(pool).await.unwrap();
    secret
}
#[sqlx::test(migrations = false)]
async fn revoked_developer_access_blocks_new_bounty_but_preserves_original_hold_return(
    pool: PgPool,
) {
    let (s, owner, recipient, _, payer, _) = fixture(&pool).await;
    publish(&pool, &s, &owner).await;
    let original = s.query(&owner, "publish").await.unwrap();
    assert!(s.authorize(&owner).await.is_ok());
    for id in [7, 9] {
        sqlx::query("UPDATE core_identity.developer_access SET granted=FALSE,revision=revision+1 WHERE user_id=$1").bind(id).execute(&pool).await.unwrap();
    }
    assert!(matches!(s.authorize(&owner).await, Err(Error::Forbidden)));
    assert!(matches!(
        s.session_authorize(&recipient).await,
        Err(Error::Forbidden)
    ));
    let owner = synthetic_reauthenticated_session(&pool, 7).await;
    let recipient = synthetic_reauthenticated_session(&pool, 9).await;
    let ids = IdentityStore::from_pool(pool.clone());
    assert!(ids.authorize(&owner).await.is_ok());
    assert!(ids.authorize(&recipient).await.is_ok());
    assert!(s.session_authorize(&owner).await.is_ok());
    assert!(s.session_authorize(&recipient).await.is_ok());
    for (token, action) in [
        (&owner, "publish"),
        (&recipient, "accept"),
        (&owner, "tip"),
        (&owner, "pause"),
    ] {
        assert!(matches!(s.authorize(token).await, Err(Error::Forbidden)));
        let mut c = cmd(&format!("revoked-{action}"), action);
        c.reward_credit = 10001;
        c.reward_slots = 1;
        c.repository_url = "https://github.com/synthetic/bounty".into();
        c.github_handle = "synthetic".into();
        c.credit = 10;
        let before = financial_snapshot(&pool).await;
        assert!(matches!(s.preview(token, &c).await, Err(Error::Forbidden)));
        assert!(matches!(s.execute(token, &c).await, Err(Error::Forbidden)));
        assert_eq!(before, financial_snapshot(&pool).await);
    }
    assert_eq!(s.query(&owner, "publish").await.unwrap(), original);
    let mut close = cmd("revoked-protected-close", "close");
    approved(&pool, &s, &owner, &mut close).await;
    assert_eq!(s.execute(&owner, &close).await.unwrap().credit, 19800);
    assert_eq!(
        Ledger::from_pool(pool)
            .balance(payer)
            .await
            .unwrap()
            .reserved_units,
        0
    );
}
#[sqlx::test(migrations = false)]
async fn public_fee_config_reports_exact_policy_or_disabled_without_invented_default(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let exists: bool = sqlx::query_scalar("SELECT to_regnamespace('core_bounties') IS NOT NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    if !exists {
        sqlx::raw_sql(bounties::SCHEMA)
            .execute(&pool)
            .await
            .unwrap();
    }
    let s = BountyStore::from_pool(pool.clone());
    let config = s.fee_config().await.unwrap();
    assert!(!config.enabled);
    assert_eq!(config.code, "policy_not_configured");
    assert_eq!(config.rate_basis_points, 0);
    let ids = IdentityStore::from_pool(pool.clone());
    let admin = ids.bootstrap_user(11, 6).await.unwrap().secret;
    let account = ids.authorize(&admin).await.unwrap().owner_account_id;
    sqlx::query("INSERT INTO core_bounties.policy(id,fee_bps,fee_account_id) VALUES(TRUE,123,$1)")
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
    let config = s.fee_config().await.unwrap();
    assert!(config.enabled);
    assert_eq!(config.rate_basis_points, 123);
    assert_eq!(config.rate_percent, "1.23");
    assert_eq!(config.limits.reward_slots_max, 100);
}
#[sqlx::test(migrations = false)]
async fn initial_project_insert_cannot_freeze_another_payers_reservation(pool: PgPool) {
    use sha2::{Digest, Sha256};
    let (s, owner, _, _, _, _) = fixture(&pool).await;
    publish(&pool, &s, &owner).await;
    let ids = IdentityStore::from_pool(pool.clone());
    let attacker = ids.bootstrap_user(13, 1).await.unwrap().secret;
    let payer = ids.authorize(&attacker).await.unwrap().owner_account_id;
    let mut c = cmd("foreign-hold-attack", "publish");
    c.project_id = "forged-foreign-project".into();
    c.reward_credit = 10001;
    c.reward_slots = 2;
    c.repository_url = "https://github.com/synthetic/bounty".into();
    approved(&pool, &s, &attacker, &mut c).await;
    let mut body: serde_json::Value =
        sqlx::query_scalar("SELECT body FROM core_bounties.projects WHERE id='synthetic-project'")
            .fetch_one(&pool)
            .await
            .unwrap();
    body["owner"] = 13.into();
    body["payer"] = payer.into();
    body["publication_operation"] = c.operation_id.clone().into();
    body["consumption"] = format!(
        "bounty:{:x}",
        Sha256::digest(serde_json::to_vec(&c.operation_id).unwrap())
    )
    .into();
    let before = financial_snapshot(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    let t: i64 = sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    body["published_at"] = t.into();
    let error = sqlx::query("INSERT INTO core_bounties.projects(id,body) VALUES($1,$2)")
        .bind(&c.project_id)
        .bind(body)
        .execute(&mut *tx)
        .await
        .unwrap_err();
    let db = error.as_database_error().unwrap();
    assert_eq!(db.code().as_deref(), Some("23514"));
    assert!(
        db.message().contains("original newly posted hold"),
        "{db:?}"
    );
    tx.rollback().await.unwrap();
    assert_eq!(before, financial_snapshot(&pool).await);
}
#[tokio::test]
async fn confirmation_http_override_accepts_only_explicit_canonical_numeric_loopback() {
    use lmm_core::bounties::http::{confirm_router, confirm_router_with_local_http};
    let p = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://synthetic:synthetic@127.0.0.1/synthetic")
        .unwrap();
    let s = BountyStore::from_pool(p);
    for origin in ["http://127.0.0.1:4317", "http://[::1]:4317"] {
        assert!(confirm_router_with_local_http(s.clone(), origin.into(), true).is_ok());
        assert!(confirm_router(s.clone(), origin.into()).is_err());
    }
    for origin in [
        "http://localhost:4317",
        "http://127.1:4317",
        "http://127.0.0.2:4317",
        "http://127.0.0.1",
        "http://[::1]",
        "http://0.0.0.0:4317",
        "http://127.0.0.1:4317/path",
        "http://user@127.0.0.1:4317",
    ] {
        assert!(
            confirm_router_with_local_http(s.clone(), origin.into(), true).is_err(),
            "{origin}"
        );
    }
    assert!(confirm_router(s, "https://bounty.invalid".into()).is_ok());
}
