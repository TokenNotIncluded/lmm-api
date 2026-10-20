//! Original main 3948a0a apps/api-go/model/tool_market_reports.go and
//! tool_market_queries.go, AGPL-3.0-or-later. Reports retain billing evidence,
//! never arguments or supplier secrets. Review suspends; it never refunds.
use super::{
    credit_access::{CreditStore, ListRequest},
    credit_lifecycle::{LifecycleReceipt, LifecycleRequest},
    credits, *,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReportRequest {
    pub call_id: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewReportRequest {
    pub call_id: String,
    pub confirmed: bool,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportListRequest {
    pub offset: i64,
    pub limit: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportProofSource {
    SupplierSigned,
    HostWitness,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BillingEvidence {
    pub version_id: String,
    pub tool_id: String,
    pub billing_price: credits::Price,
    pub pricing_digest: String,
    pub fee_bps: i64,
    pub recipient_id: i64,
    pub held_quota: i64,
    pub charged_quota: i64,
    pub fee_quota: i64,
    pub seller_quota: i64,
    pub refunded_quota: i64,
    pub proof_source: Option<ReportProofSource>,
    pub usage_source: Option<credits::UsageSource>,
    pub usage_quantities: Option<std::collections::BTreeMap<String, i64>>,
    pub result_digest: String,
    pub settlement_status: String,
    pub settlement_journal_id: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallReport {
    pub call_id: String,
    pub user_id: i64,
    pub service_id: String,
    pub owner_id: i64,
    pub reason: String,
    pub evidence: BillingEvidence,
    pub status: String,
    pub review_note: String,
    pub reviewed_by: i64,
    pub created_at: i64,
    pub reviewed_at: i64,
    pub service_lifecycle: Option<LifecycleReceipt>,
}
#[derive(Serialize, Deserialize)]
pub struct ReportPage {
    pub rows: Vec<CallReport>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IncomeReceipt {
    pub id: String,
    pub call_id: String,
    pub kind: String,
    pub quota: i64,
    pub fee_bps: i64,
    pub recipient_id: i64,
    pub seller_id: i64,
    pub settlement_journal_id: i64,
    pub created_at: i64,
}
#[derive(Serialize, Deserialize)]
pub struct IncomePage {
    pub owner: Owner,
    pub rows: Vec<IncomeReceipt>,
}
fn text(v: &str) -> Result<String> {
    let v = v.trim();
    if v.is_empty() || v.chars().count() > 2000 {
        return Err(Error::Invalid);
    }
    Ok(v.into())
}
fn page(offset: i64, limit: i64) -> Result<()> {
    if !(0..=10000).contains(&offset) || !(1..=100).contains(&limit) {
        return Err(Error::Invalid);
    }
    Ok(())
}
async fn admin(tx: &mut Transaction<'_, Postgres>, actor: i64) -> Result<()> {
    let role: String =
        sqlx::query_scalar("SELECT platform_role FROM core_identity.users WHERE id=$1 FOR SHARE")
            .bind(actor)
            .fetch_one(&mut **tx)
            .await?;
    if !matches!(role.as_str(), "admin" | "superadmin") {
        return Err(Error::Forbidden);
    }
    if crate::identity::has_privileged_import_facts(tx, actor)
        .await
        .map_err(|_| Error::Storage)?
    {
        crate::identity::load_complete_policy(tx, actor)
            .await
            .map_err(|_| Error::Forbidden)?;
    }
    Ok(())
}
async fn report(tx: &mut Transaction<'_, Postgres>, id: &str) -> Result<Option<CallReport>> {
    let rows:Vec<Json<CallReport>>=sqlx::query_scalar("SELECT details FROM core_tool_market.credit_access_operations WHERE kind='call-report' AND target_id=$1").bind(id).fetch_all(&mut **tx).await?;
    if rows.len() > 1 {
        return Err(Error::Storage);
    }
    let Some(Json(mut out)) = rows.into_iter().next() else {
        return Ok(None);
    };
    let reviews:Vec<Json<CallReport>>=sqlx::query_scalar("SELECT details FROM core_tool_market.credit_access_operations WHERE kind='call-report-review' AND target_id=$1").bind(id).fetch_all(&mut **tx).await?;
    if reviews.len() > 1 {
        return Err(Error::Storage);
    }
    if let Some(Json(review)) = reviews.into_iter().next() {
        out = review;
    }
    Ok(Some(out))
}
async fn report_lock(tx: &mut Transaction<'_, Postgres>, service: &str, id: &str) -> Result<()> {
    // Match lifecycle/dispatch lock order: service before report.
    for lock in [
        format!("credit-service:{service}"),
        format!("credit-report:{id}"),
    ] {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(lock)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
impl CreditStore {
    pub async fn create_report(&self, secret: &str, r: &ReportRequest) -> Result<CallReport> {
        if !digest(&r.call_id) {
            return Err(Error::Invalid);
        }
        let reason = text(&r.reason)?;
        let mut tx = self.base.begin().await?;
        let actor = ToolMarketStore::actor(&mut tx, secret).await?;
        let call = sqlx::query("SELECT * FROM core_tool_market.credit_calls WHERE id=$1")
            .bind(&r.call_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Missing)?;
        if call.try_get::<i64, _>("actor_user_id")? != actor.user_id {
            return Err(Error::Forbidden);
        }
        let Json(snapshot): Json<serde_json::Value> = call.try_get("snapshot")?;
        let service = snapshot["quote"]["service_id"]
            .as_str()
            .ok_or(Error::Storage)?;
        report_lock(&mut tx, service, &r.call_id).await?;
        let state: String = call.try_get("state")?;
        if !matches!(state.as_str(), "settled" | "released") {
            return Err(Error::Conflict);
        }
        if let Some(old) = report(&mut tx, &r.call_id).await? {
            if old.user_id != actor.user_id || old.reason != reason {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return Ok(old);
        }
        let Json(receipt): Json<super::credit_execution::Receipt> = call.try_get("receipt")?;
        if receipt.state != state {
            return Err(Error::Storage);
        }
        let price: credits::Price = serde_json::from_value(snapshot["quote"]["price"].clone())
            .map_err(|_| Error::Storage)?;
        let usage: Option<Json<credits::Usage>> = call.try_get("usage")?;
        // The caller's Usage.source is metadata, never evidence of who proved
        // delivery. Only immutable Core-accepted proof records establish that.
        let (supplier, witness): (bool, bool) = sqlx::query_as("SELECT EXISTS(SELECT FROM core_tool_market.credit_supplier_receipts WHERE call_id=$1), EXISTS(SELECT FROM core_tool_market.credit_fixed_witness_receipts WHERE call_id=$1)")
            .bind(&r.call_id).fetch_one(&mut *tx).await?;
        let (proof_source, usage_source) = match (supplier, witness) {
            (true, false) => (
                Some(ReportProofSource::SupplierSigned),
                Some(credits::UsageSource::ToolReported),
            ),
            (false, true) => (Some(ReportProofSource::HostWitness), None),
            (false, false) => (None, None),
            (true, true) => return Err(Error::Storage),
        };
        let evidence = BillingEvidence {
            version_id: receipt.version_id,
            tool_id: receipt.tool_id,
            billing_price: price,
            pricing_digest: receipt.pricing_digest,
            fee_bps: snapshot["fee_bps"].as_i64().ok_or(Error::Storage)?,
            recipient_id: snapshot["platform_user_id"].as_i64().unwrap_or(0),
            held_quota: receipt.held_quota,
            charged_quota: receipt.charged_quota,
            fee_quota: receipt.fee_quota,
            seller_quota: receipt.seller_quota,
            refunded_quota: receipt.refunded_quota,
            proof_source,
            usage_source,
            usage_quantities: usage.map(|u| u.0.quantities),
            result_digest: receipt.result_digest,
            settlement_status: state,
            settlement_journal_id: call.try_get("settlement_journal_id")?,
        };
        let now = ToolMarketStore::now(&mut tx).await?;
        let out = CallReport {
            call_id: r.call_id.clone(),
            user_id: actor.user_id,
            service_id: service.into(),
            owner_id: snapshot["seller_user_id"].as_i64().unwrap_or(0),
            reason: reason.clone(),
            evidence,
            status: "pending".into(),
            review_note: String::new(),
            reviewed_by: 0,
            created_at: now,
            reviewed_at: 0,
            service_lifecycle: None,
        };
        Self::record(
            &mut tx,
            actor.user_id,
            &format!("report-{}", r.call_id),
            "call-report",
            &ReportRequest {
                call_id: r.call_id.clone(),
                reason,
            },
            &r.call_id,
            now,
            Some(serde_json::to_value(&out).map_err(|_| Error::Storage)?),
        )
        .await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn review_report(&self, secret: &str, r: &ReviewReportRequest) -> Result<CallReport> {
        if !digest(&r.call_id) {
            return Err(Error::Invalid);
        }
        let note = text(&r.note)?;
        let mut tx = self.base.begin().await?;
        let actor = ToolMarketStore::actor(&mut tx, secret).await?;
        admin(&mut tx, actor.user_id).await?;
        let original = report(&mut tx, &r.call_id).await?.ok_or(Error::Missing)?;
        report_lock(&mut tx, &original.service_id, &r.call_id).await?;
        let mut out = report(&mut tx, &r.call_id).await?.ok_or(Error::Missing)?;
        if out.user_id == actor.user_id || out.owner_id == actor.user_id {
            return Err(Error::Forbidden);
        }
        let status = if r.confirmed {
            "confirmed"
        } else {
            "dismissed"
        };
        if out.status != "pending" {
            if out.status != status || out.reviewed_by != actor.user_id || out.review_note != note {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return Ok(out);
        }
        let now = ToolMarketStore::now(&mut tx).await?;
        if r.confirmed {
            if out.owner_id <= 0 {
                return Err(Error::Forbidden);
            }
            let row=sqlx::query("SELECT * FROM core_tool_market.credit_services WHERE service_id=$1 AND owner_user_id=$2 FOR UPDATE").bind(&out.service_id).bind(out.owner_id).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
            if !row.try_get::<bool, _>("deleted")? {
                let revision = row
                    .try_get::<i64, _>("revision")?
                    .checked_add(1)
                    .ok_or(Error::Invalid)?;
                sqlx::query("UPDATE core_tool_market.credit_services SET paused=TRUE,suspended=TRUE,revision=$3,updated_by=$4,updated_at=$5 WHERE service_id=$1 AND owner_user_id=$2").bind(&out.service_id).bind(out.owner_id).bind(revision).bind(actor.user_id).bind(now).execute(&mut *tx).await?;
                out.service_lifecycle = Some(LifecycleReceipt {
                    request: LifecycleRequest {
                        operation_id: format!("review-{}", r.call_id),
                        service_id: out.service_id.clone(),
                        paused: true,
                        deleted: false,
                    },
                    suspended: true,
                    revision,
                    updated_by: actor.user_id,
                    updated_at: now,
                });
            }
        }
        out.status = status.into();
        out.review_note = note.clone();
        out.reviewed_by = actor.user_id;
        out.reviewed_at = now;
        Self::record(
            &mut tx,
            actor.user_id,
            &format!("review-{}", r.call_id),
            "call-report-review",
            &ReviewReportRequest {
                call_id: r.call_id.clone(),
                confirmed: r.confirmed,
                note,
            },
            &r.call_id,
            now,
            Some(serde_json::to_value(&out).map_err(|_| Error::Storage)?),
        )
        .await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn list_reports(&self, secret: &str, r: &ReportListRequest) -> Result<ReportPage> {
        page(r.offset, r.limit)?;
        let mut tx = self.base.begin().await?;
        let actor = ToolMarketStore::actor(&mut tx, secret).await?;
        admin(&mut tx, actor.user_id).await?;
        let rows:Vec<Json<CallReport>>=sqlx::query_scalar("SELECT COALESCE(v.details,c.details) AS details FROM core_tool_market.credit_access_operations c LEFT JOIN core_tool_market.credit_access_operations v ON v.target_id=c.target_id AND v.kind='call-report-review' WHERE c.kind='call-report' AND (c.details->>'user_id')::BIGINT<>$1 AND (c.details->>'owner_id')::BIGINT<>$1 ORDER BY CASE WHEN v.target_id IS NULL THEN 0 ELSE 1 END,c.created_at DESC,c.target_id ASC OFFSET $2 LIMIT $3").bind(actor.user_id).bind(r.offset).bind(r.limit).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(ReportPage {
            rows: rows.into_iter().map(|r| r.0).collect(),
        })
    }
    pub async fn list_income(&self, secret: &str, r: &ListRequest) -> Result<IncomePage> {
        page(r.offset, r.limit)?;
        let mut tx = self.base.begin().await?;
        let actor = Self::owned(&mut tx, secret, &r.owner).await?;
        // Only persisted settled calls with a real original split-capture journal
        // contribute income. Policy changes cannot rewrite these frozen parties.
        let records=sqlx::query("SELECT c.id,c.snapshot,c.receipt,c.settlement_journal_id,c.reserve_journal_id,j.parent_journal_id,j.kind,floor(extract(epoch FROM j.created_at))::BIGINT AS settled_at,o.request->'action' AS action,x.kind AS income_kind,x.quota,x.recipient FROM core_tool_market.credit_calls c JOIN core_billing.ledger_journals j ON j.id=c.settlement_journal_id JOIN core_billing.ledger_operations o ON o.scope=j.scope AND o.operation_key=j.operation_key CROSS JOIN LATERAL (VALUES ('author', (c.receipt->>'seller_quota')::BIGINT,(c.snapshot->>'seller_user_id')::BIGINT),('platform',(c.receipt->>'fee_quota')::BIGINT,(c.snapshot->>'platform_user_id')::BIGINT)) x(kind,quota,recipient) WHERE c.state='settled' AND x.recipient=$1 AND x.quota>0 ORDER BY j.created_at DESC,c.id ASC,x.kind ASC OFFSET $2 LIMIT $3").bind(actor.user_id).bind(r.offset).bind(r.limit).fetch_all(&mut *tx).await?;
        let mut rows = Vec::new();
        for row in records {
            let Json(s): Json<serde_json::Value> = row.try_get("snapshot")?;
            let Json(a): Json<serde_json::Value> = row.try_get("action")?;
            let Json(receipt): Json<super::credit_execution::Receipt> = row.try_get("receipt")?;
            let fee = s["fee_bps"].as_i64().ok_or(Error::Storage)?;
            if row.try_get::<String, _>("kind")? != "split_commerce_capture"
                || a["kind"] != "split_commerce_capture"
                || a["seller_account_id"] != s["seller_account_id"]
                || a["platform_account_id"] != s["platform_account_id"]
                || a["fee_basis_points"].as_i64() != Some(fee)
                || a["amount_units"].as_i64() != Some(receipt.charged_quota)
                || a["reservation_journal_id"].as_i64()
                    != row.try_get::<Option<i64>, _>("reserve_journal_id")?
                || row.try_get::<Option<i64>, _>("parent_journal_id")?
                    != row.try_get::<Option<i64>, _>("reserve_journal_id")?
                || receipt.charged_quota < 0
                || receipt.charged_quota > credits::MAX_CREDITS
                || receipt.fee_quota
                    != ((i128::from(receipt.charged_quota) * i128::from(fee)) / 10000) as i64
                || receipt.seller_quota != receipt.charged_quota - receipt.fee_quota
                || receipt.refunded_quota != receipt.held_quota - receipt.charged_quota
                || receipt.state != "settled"
                || !(0..=10000).contains(&fee)
            {
                return Err(Error::Storage);
            }
            let id: String = row.try_get("id")?;
            let kind: String = row.try_get("income_kind")?;
            rows.push(IncomeReceipt {
                id: format!("{id}:{kind}"),
                call_id: id,
                kind,
                quota: row.try_get("quota")?,
                fee_bps: fee,
                recipient_id: row.try_get("recipient")?,
                seller_id: s["seller_user_id"].as_i64().ok_or(Error::Storage)?,
                settlement_journal_id: row.try_get("settlement_journal_id")?,
                created_at: row.try_get("settled_at")?,
            });
        }
        tx.commit().await?;
        Ok(IncomePage {
            owner: r.owner.clone(),
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        credit_access::{ApproveRequest, ConsentRequest},
        credit_config::ConfigRequest,
        credit_execution::{ReleaseRequest, ReserveRequest, SettleRequest},
    };
    use super::*;
    use crate::identity::IdentityStore;
    use base64::Engine as _;
    struct Fixture {
        store: CreditStore,
        buyer: String,
        seller: String,
        root: String,
        reviewer: String,
        imported: String,
        other: String,
        quote: credits::Quote,
        grant: String,
        now: i64,
        buyer_wallet: i64,
        seller_wallet: i64,
        root_wallet: i64,
    }
    async fn fixture(pool: &PgPool) -> Fixture {
        fixture_schema(pool, serde_json::json!({"type":"object"})).await
    }
    async fn fixture_schema(pool: &PgPool, schema: serde_json::Value) -> Fixture {
        crate::core_schema::init(pool).await.unwrap();
        let exists: bool =
            sqlx::query_scalar("SELECT to_regclass('core_tool_market.credit_calls') IS NOT NULL")
                .fetch_one(pool)
                .await
                .unwrap();
        if !exists {
            sqlx::raw_sql(credits::SCHEMA_V2)
                .execute(pool)
                .await
                .unwrap();
        }
        let identity = IdentityStore::from_pool(pool.clone());
        let mut secrets = Vec::new();
        let mut wallets = Vec::new();
        for (user, role) in [(7, 5), (9, 5), (11, 6), (13, 5), (15, 5), (17, 5)] {
            let issued = identity.bootstrap_user(user, role).await.unwrap();
            wallets.push(
                identity
                    .authorize(&issued.secret)
                    .await
                    .unwrap()
                    .owner_account_id,
            );
            secrets.push(issued.secret);
        }
        let ledger = crate::billing::ledger::Ledger::from_pool(pool.clone());
        for id in &wallets {
            ledger.open_wallet(*id).await.unwrap();
        }
        assert!(matches!(
            ledger
                .execute(&Request {
                    scope: "reports-fixture".into(),
                    operation_key: "fund-buyer".into(),
                    actor_user_id: Some(7),
                    reason: "Synthetic report acceptance funds".into(),
                    action: Action::Credit {
                        account_id: wallets[0],
                        amount_units: Credits::try_from(500).unwrap(),
                        payment_reference: "reports-synthetic-only".into()
                    }
                })
                .await
                .unwrap(),
            Outcome::Posted { .. }
        ));
        let store = CreditStore::from_pool(pool.clone());
        store
            .update_config(
                &secrets[2],
                &ConfigRequest {
                    operation_id: "reports-policy".into(),
                    enabled: true,
                    fee_bps: 2500,
                    recipient_id: 11,
                },
            )
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_tool_market.publishers(user_id,active) VALUES(9,TRUE)")
            .execute(pool)
            .await
            .unwrap();
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::BIGINT")
                .fetch_one(pool)
                .await
                .unwrap();
        let price = credits::Price::Fixed { price_quota: 80 };
        let pricing = hash(&price).unwrap();
        let remote = "a".repeat(64);
        let quote = credits::Quote {
            id: hash(&[
                "legacy-credit-quote-v2",
                "reports-version",
                "reports-tool",
                &remote,
                &pricing,
            ])
            .unwrap(),
            service_id: "reports-service".into(),
            tool_id: "reports-tool".into(),
            version_id: "reports-version".into(),
            remote_digest: remote,
            pricing_digest: pricing,
            price,
            expires: now + 3600,
        };
        store.register_quote(&secrets[1], &quote).await.unwrap();
        store
            .approve_quote(
                &secrets[4],
                &ApproveRequest {
                    operation_id: "approve-reports".into(),
                    quote_id: quote.id.clone(),
                    visibility: "public".into(),
                    approved: true,
                },
            )
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_tool_market.credit_result_contracts(quote_id,proof_mode,witness_role,output_schema,output_schema_digest) VALUES($1,'core_host_fixed_success','reports-fixture-witness',$2,$3)").bind(&quote.id).bind(Json(&schema)).bind(hash(&schema).unwrap()).execute(pool).await.unwrap();
        let grant = store
            .create_consent(
                &secrets[0],
                &ConsentRequest {
                    owner: Owner {
                        user: "7".into(),
                        account: "7".into(),
                    },
                    operation_id: "reports-consent".into(),
                    client_id: "web-market".into(),
                    tool_id: quote.tool_id.clone(),
                    version_id: quote.version_id.clone(),
                    max_price_quota: 80,
                    max_total_quota: 800,
                    max_calls: 10,
                    expires_at: now + 3500,
                },
            )
            .await
            .unwrap();
        // Imported admin exists but has NO complete policy; its role alone cannot review.
        sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES(13,$1,$1,$1,$1,'reports-import-fixture',$1,1,$1,$1,TRUE,FALSE,TRUE,TRUE,$1,$1,'admin',FALSE,1)").bind(vec![1_u8;32]).execute(pool).await.unwrap();
        Fixture {
            store,
            buyer: secrets[0].clone(),
            seller: secrets[1].clone(),
            root: secrets[2].clone(),
            imported: secrets[3].clone(),
            reviewer: secrets[4].clone(),
            other: secrets[5].clone(),
            quote,
            grant: grant.id,
            now,
            buyer_wallet: wallets[0],
            seller_wallet: wallets[1],
            root_wallet: wallets[2],
        }
    }
    impl Fixture {
        async fn reserve(&self, key: &str) -> super::super::credit_execution::Receipt {
            self.store
                .reserve(
                    &self.buyer,
                    &ReserveRequest {
                        owner: Owner {
                            user: "7".into(),
                            account: "7".into(),
                        },
                        client_id: "web-market".into(),
                        request_key: key.into(),
                        quote_id: self.quote.id.clone(),
                        grant_id: self.grant.clone(),
                        tool_id: self.quote.tool_id.clone(),
                        version_id: self.quote.version_id.clone(),
                        input_digest: "b".repeat(64),
                        resolve_by: self.now + 1800,
                    },
                )
                .await
                .unwrap()
        }
        async fn release(&self, id: &str) {
            self.store
                .release(&ReleaseRequest {
                    raw_result: None,
                    call_id: id.into(),
                    dispatch_id: String::new(),
                    reason: "cancelled".into(),
                    evidence_digest: String::new(),
                })
                .await
                .unwrap();
        }
        async fn settle(&self, id: &str) {
            let dispatched = self.store.dispatch(&self.buyer, id).await.unwrap();
            let result = serde_json::json!({"content":[{"type":"text","text":"private supplier answer not report evidence"}],"structuredContent":{}});
            let raw = serde_json::to_vec(&result).unwrap();
            self.store
                .settle_fixed_witness(
                    &SettleRequest {
                        call_id: id.into(),
                        dispatch_id: dispatched.dispatch_id,
                        result_digest: super::super::receipt_verifier::result_digest(&result)
                            .unwrap(),
                        usage: None,
                        raw_result: Some(base64::engine::general_purpose::STANDARD.encode(raw)),
                    },
                    "reports-fixture-witness",
                )
                .await
                .unwrap();
        }
    }
    async fn financial(pool: &PgPool) -> serde_json::Value {
        sqlx::query_scalar("SELECT jsonb_build_object('journals',(SELECT coalesce(jsonb_agg(to_jsonb(j) ORDER BY id),'[]') FROM core_billing.ledger_journals j),'entries',(SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY journal_id,ledger_account_id),'[]') FROM core_billing.ledger_entries e),'balances',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id),'[]') FROM core_billing.balance_state b))").fetch_one(pool).await.unwrap()
    }
    fn list(user: i64) -> ListRequest {
        ListRequest {
            owner: Owner {
                user: user.to_string(),
                account: user.to_string(),
            },
            offset: 0,
            limit: 100,
        }
    }
    #[sqlx::test(migrations = false)]
    async fn reports_terminal_unique_nonparty_admin_suspend_is_atomic_and_nonfinancial(
        pool: PgPool,
    ) {
        let f = fixture(&pool).await;
        let call = f.reserve("report-call").await;
        // Same raw service id in another owner's namespace is unrelated.
        sqlx::query("INSERT INTO core_tool_market.credit_services(service_id,owner_user_id,updated_by,updated_at) VALUES('reports-service',17,17,$1)").bind(f.now).execute(&pool).await.unwrap();
        let req = ReportRequest {
            call_id: call.call_id.clone(),
            reason: "  wrong delivery  ".into(),
        };
        assert!(matches!(
            f.store.create_report(&f.buyer, &req).await,
            Err(Error::Conflict)
        ));
        f.release(&call.call_id).await;
        assert!(matches!(
            f.store.create_report(&f.seller, &req).await,
            Err(Error::Forbidden)
        ));
        let report = f.store.create_report(&f.buyer, &req).await.unwrap();
        assert_eq!(report.reason, "wrong delivery");
        assert_eq!(report.evidence.settlement_status, "released");
        assert_eq!(report.evidence.refunded_quota, 80);
        assert_eq!(report.evidence.proof_source, None);
        assert_eq!(report.evidence.usage_source, None);
        assert_eq!(
            f.store
                .create_report(&f.buyer, &req)
                .await
                .unwrap()
                .created_at,
            report.created_at
        );
        assert!(matches!(
            f.store
                .create_report(
                    &f.buyer,
                    &ReportRequest {
                        reason: "changed".into(),
                        ..req.clone()
                    }
                )
                .await,
            Err(Error::Conflict)
        ));
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_tool_market.credit_access_operations WHERE kind='call-report'").fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);
        let page = ReportListRequest {
            offset: 0,
            limit: 100,
        };
        for secret in [&f.buyer, &f.seller] {
            assert!(
                f.store
                    .list_reports(secret, &page)
                    .await
                    .unwrap()
                    .rows
                    .is_empty()
            );
        }
        assert!(matches!(
            f.store.list_reports(&f.imported, &page).await,
            Err(Error::Forbidden)
        ));
        assert_eq!(
            f.store
                .list_reports(&f.reviewer, &page)
                .await
                .unwrap()
                .rows
                .len(),
            1
        );
        let review = ReviewReportRequest {
            call_id: call.call_id.clone(),
            confirmed: true,
            note: "verified original evidence".into(),
        };
        for secret in [&f.buyer, &f.seller, &f.imported] {
            assert!(matches!(
                f.store.review_report(secret, &review).await,
                Err(Error::Forbidden)
            ));
        }
        // A verified_complete header alone does not authorize an imported admin.
        let entries: Vec<_> = crate::identity::Action::CATALOG
            .iter()
            .map(|a| (a.catalog_name().unwrap().to_string(), "allow".to_string()))
            .collect();
        let policy = crate::identity::complete_policy_snapshot(13, 1, &entries).unwrap();
        let payload = crate::identity::canonical_policy_payload(&policy).unwrap();
        sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES(13,1,'core.permissions.v1',$1,$2,TRUE)")
            .bind(crate::identity::policy_sha256(&policy).unwrap().to_vec()).bind(payload).execute(&pool).await.unwrap();
        assert!(matches!(
            f.store.list_reports(&f.imported, &page).await,
            Err(Error::Forbidden)
        ));
        for (action, effect) in entries {
            sqlx::query("INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES(13,$1,$2)").bind(action).bind(effect).execute(&pool).await.unwrap();
        }
        assert_eq!(
            f.store
                .list_reports(&f.imported, &page)
                .await
                .unwrap()
                .rows
                .len(),
            1
        );
        // An ordinary API key must never become a Browser Session review actor.
        let identity = IdentityStore::from_pool(pool.clone());
        let key = identity
            .issue_key(
                &f.reviewer,
                crate::accounts::Account {
                    kind: crate::accounts::AccountKind::Personal,
                    id: 15,
                },
                crate::funding::FundingPolicy::new_personal_key(),
                3600,
            )
            .await
            .unwrap();
        assert_eq!(
            identity
                .authorize(&key.secret)
                .await
                .unwrap()
                .credential_kind,
            crate::identity::CredentialKind::ApiKey
        );
        assert!(matches!(
            f.store.list_reports(&key.secret, &page).await,
            Err(Error::Forbidden)
        ));
        let before = financial(&pool).await;
        // Abort AFTER lifecycle UPDATE: both report decision and suspension must rollback.
        sqlx::raw_sql("CREATE FUNCTION public.report_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.kind='call-report-review' THEN RAISE EXCEPTION 'synthetic report persistence failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER report_failure BEFORE INSERT ON core_tool_market.credit_access_operations FOR EACH ROW EXECUTE FUNCTION public.report_failure();").execute(&pool).await.unwrap();
        assert!(f.store.review_report(&f.reviewer, &review).await.is_err());
        let paused:bool=sqlx::query_scalar("SELECT paused OR suspended FROM core_tool_market.credit_services WHERE service_id='reports-service' AND owner_user_id=9").fetch_one(&pool).await.unwrap();
        assert!(!paused);
        sqlx::raw_sql("DROP TRIGGER report_failure ON core_tool_market.credit_access_operations; DROP FUNCTION public.report_failure();").execute(&pool).await.unwrap();
        let done = f.store.review_report(&f.reviewer, &review).await.unwrap();
        assert_eq!(done.status, "confirmed");
        assert!(done.service_lifecycle.unwrap().suspended);
        let unrelated_paused:bool=sqlx::query_scalar("SELECT paused OR suspended FROM core_tool_market.credit_services WHERE service_id='reports-service' AND owner_user_id=17").fetch_one(&pool).await.unwrap();
        assert!(
            !unrelated_paused,
            "review cannot suspend another owner's service with the same raw id"
        );
        assert_eq!(financial(&pool).await, before);
        assert_eq!(
            f.store
                .review_report(&f.reviewer, &review)
                .await
                .unwrap()
                .reviewed_by,
            15
        );
        assert!(matches!(
            f.store.review_report(&f.other, &review).await,
            Err(Error::Conflict)
        ));
        let mut tx = pool.begin().await.unwrap();
        assert!(matches!(
            CreditStore::check_service(&mut tx, 9, "reports-service").await,
            Err(Error::Forbidden)
        ));
        tx.rollback().await.unwrap();
        assert!(matches!(
            f.store
                .lifecycle(
                    &f.seller,
                    &LifecycleRequest {
                        operation_id: "seller-resume".into(),
                        service_id: "reports-service".into(),
                        paused: false,
                        deleted: false
                    }
                )
                .await,
            Err(Error::Forbidden)
        ));
    }
    #[sqlx::test(migrations = false)]
    async fn income_uses_original_split_journal_and_frozen_recipients_after_policy_change(
        pool: PgPool,
    ) {
        let f = fixture(&pool).await;
        let call = f.reserve("income-call").await;
        f.settle(&call.call_id).await;
        let seller = f.store.list_income(&f.seller, &list(9)).await.unwrap();
        assert_eq!(seller.rows.len(), 1);
        assert_eq!(
            (
                seller.rows[0].kind.as_str(),
                seller.rows[0].quota,
                seller.rows[0].fee_bps,
                seller.rows[0].recipient_id
            ),
            ("author", 60, 2500, 9)
        );
        let platform = f.store.list_income(&f.root, &list(11)).await.unwrap();
        assert_eq!(
            (platform.rows[0].kind.as_str(), platform.rows[0].quota),
            ("platform", 20)
        );
        assert!(
            f.store
                .list_income(&f.buyer, &list(7))
                .await
                .unwrap()
                .rows
                .is_empty()
        );
        assert!(matches!(
            f.store.list_income(&f.buyer, &list(9)).await,
            Err(Error::Forbidden)
        ));
        let identity = IdentityStore::from_pool(pool.clone());
        let next = identity.bootstrap_user(21, 6).await.unwrap().secret;
        f.store
            .update_config(
                &f.root,
                &ConfigRequest {
                    operation_id: "change-after-income".into(),
                    enabled: true,
                    fee_bps: 7000,
                    recipient_id: 21,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            f.store.list_income(&f.seller, &list(9)).await.unwrap().rows[0].quota,
            60
        );
        assert_eq!(
            f.store.list_income(&f.root, &list(11)).await.unwrap().rows[0].fee_bps,
            2500
        );
        assert!(
            f.store
                .list_income(&next, &list(21))
                .await
                .unwrap()
                .rows
                .is_empty()
        );
        let ledger = crate::billing::ledger::Ledger::from_pool(pool.clone());
        assert_eq!(
            ledger
                .balance(f.buyer_wallet)
                .await
                .unwrap()
                .available_units,
            420
        );
        assert_eq!(
            ledger
                .balance(f.seller_wallet)
                .await
                .unwrap()
                .available_units,
            60
        );
        assert_eq!(
            ledger.balance(f.root_wallet).await.unwrap().available_units,
            20
        );
        let before = financial(&pool).await;
        let report = f
            .store
            .create_report(
                &f.buyer,
                &ReportRequest {
                    call_id: call.call_id.clone(),
                    reason: "billing evidence review".into(),
                },
            )
            .await
            .unwrap();
        let raw = serde_json::to_string(&report).unwrap();
        assert!(!raw.contains("private supplier answer"));
        assert!(!raw.contains(&f.buyer));
        assert_eq!(
            report.evidence.proof_source,
            Some(ReportProofSource::HostWitness)
        );
        assert_eq!(report.evidence.usage_source, None);
        assert_eq!(report.evidence.recipient_id, 11);
        assert_eq!(report.evidence.fee_bps, 2500);
        f.store
            .review_report(
                &f.reviewer,
                &ReviewReportRequest {
                    call_id: call.call_id,
                    confirmed: false,
                    note: "evidence retained without suspension".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(financial(&pool).await, before);
    }
    #[sqlx::test(migrations = false)]
    async fn fixed_witness_http_requires_separate_role_and_original_result(pool: PgPool) {
        use axum::{
            body::Body,
            http::{Request as HttpRequest, StatusCode},
        };
        use tower::ServiceExt;
        async fn post(
            router: &axum::Router,
            token: &str,
            path: &str,
            body: serde_json::Value,
        ) -> StatusCode {
            router
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri(format!("/internal/tool-market/v2/fixed-witness/{path}"))
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {token}"))
                        .header("x-lmm-protocol", "2")
                        .body(Body::from(serde_json::to_vec(&body).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
        async fn state(pool: &PgPool) -> serde_json::Value {
            let counters:serde_json::Value=sqlx::query_scalar("SELECT jsonb_build_object('consents',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY id),'[]') FROM core_tool_market.credit_consents c),'budgets',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY actor_user_id,scope,scope_id),'[]') FROM core_tool_market.credit_budgets b),'calls',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY id),'[]') FROM core_tool_market.credit_calls c),'proofs',(SELECT count(*) FROM core_tool_market.credit_fixed_witness_receipts))").fetch_one(pool).await.unwrap();
            serde_json::json!({"ledger":financial(pool).await,"counters":counters})
        }
        fn settle_body(
            call: &super::super::credit_execution::Receipt,
            result: &serde_json::Value,
        ) -> serde_json::Value {
            serde_json::json!({"call_id":call.call_id,"dispatch_id":call.dispatch_id,
                "result_digest":super::super::receipt_verifier::result_digest(result).unwrap(),
                "usage":null,"raw_result":base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(result).unwrap())})
        }
        let f = fixture_schema(&pool, serde_json::json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}},"additionalProperties":false})).await;
        let ordinary = "o".repeat(32);
        let purpose = "p".repeat(32);
        let router = super::super::fixed_witness_http::router(
            f.store.clone(),
            ordinary.as_bytes(),
            purpose.as_bytes(),
            "reports-fixture-witness",
        )
        .unwrap();
        let reserved = f.reserve("purpose-success").await;
        let call = f.store.dispatch(&f.buyer, &reserved.call_id).await.unwrap();
        let good = serde_json::json!({"content":[{"type":"text","text":"synthetic success"}],"structuredContent":{"ok":true}});
        let body = settle_body(&call, &good);
        let before = state(&pool).await;
        assert_eq!(
            post(&router, &ordinary, "settle", body.clone()).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(state(&pool).await, before);
        for invalid in [
            serde_json::json!({"content":[1],"structuredContent":{"ok":true}}),
            serde_json::json!({"content":[{"type":"text","text":"wrong schema"}],"structuredContent":{"ok":"wrong"}}),
        ] {
            assert!(
                !post(&router, &purpose, "settle", settle_body(&call, &invalid))
                    .await
                    .is_success()
            );
            assert_eq!(state(&pool).await, before);
        }
        assert_eq!(
            post(&router, &purpose, "settle", body.clone()).await,
            StatusCode::OK
        );
        let once = state(&pool).await;
        assert_eq!(
            post(&router, &purpose, "settle", body.clone()).await,
            StatusCode::OK
        );
        assert_eq!(state(&pool).await, once);
        let mut changed = body;
        changed["raw_result"]=serde_json::json!(base64::engine::general_purpose::STANDARD.encode(br#"{"content":[{"type":"text","text":"changed"}],"structuredContent":{"ok":true}}"#));
        assert!(
            !post(&router, &purpose, "settle", changed)
                .await
                .is_success()
        );
        assert_eq!(state(&pool).await, once);
        let proof_count:i64=sqlx::query_scalar("SELECT count(*) FROM core_tool_market.credit_fixed_witness_receipts WHERE call_id=$1 AND outcome='success'").bind(&call.call_id).fetch_one(&pool).await.unwrap();
        assert_eq!(proof_count, 1);
        let ledger = crate::billing::ledger::Ledger::from_pool(pool.clone());
        assert_eq!(
            ledger
                .balance(f.buyer_wallet)
                .await
                .unwrap()
                .available_units,
            420
        );
        assert_eq!(
            ledger
                .balance(f.seller_wallet)
                .await
                .unwrap()
                .available_units,
            60
        );
        assert_eq!(
            ledger.balance(f.root_wallet).await.unwrap().available_units,
            20
        );
        let reserved = f.reserve("purpose-failure").await;
        let failed = f.store.dispatch(&f.buyer, &reserved.call_id).await.unwrap();
        let unknown = serde_json::json!({"content":[{"type":"text","text":"unknown status"}],"status":"unknown"});
        let release = |result: &serde_json::Value| {
            serde_json::json!({"call_id":failed.call_id,"dispatch_id":failed.dispatch_id,"reason":"failed",
            "evidence_digest":super::super::receipt_verifier::result_digest(result).unwrap(),
            "raw_result":base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(result).unwrap())})
        };
        let before = state(&pool).await;
        assert!(
            !post(&router, &purpose, "release", release(&unknown))
                .await
                .is_success()
        );
        assert_eq!(state(&pool).await, before);
        let failure = serde_json::json!({"isError":true,"content":[{"type":"text","text":"synthetic explicit failure"}]});
        assert_eq!(
            post(&router, &ordinary, "release", release(&failure)).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(state(&pool).await, before);
        assert_eq!(
            post(&router, &purpose, "release", release(&failure)).await,
            StatusCode::OK
        );
        let once = state(&pool).await;
        assert_eq!(
            post(&router, &purpose, "release", release(&failure)).await,
            StatusCode::OK
        );
        assert_eq!(state(&pool).await, once);
        let proof_count:i64=sqlx::query_scalar("SELECT count(*) FROM core_tool_market.credit_fixed_witness_receipts WHERE call_id=$1 AND outcome='failure'").bind(&failed.call_id).fetch_one(&pool).await.unwrap();
        assert_eq!(proof_count, 1);
        assert_eq!(
            ledger
                .balance(f.buyer_wallet)
                .await
                .unwrap()
                .available_units,
            420
        );
        assert_eq!(
            ledger
                .balance(f.seller_wallet)
                .await
                .unwrap()
                .available_units,
            60
        );
        assert_eq!(
            ledger.balance(f.root_wallet).await.unwrap().available_units,
            20
        );
    }
}
