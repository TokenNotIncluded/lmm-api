use super::{event_error, policy::*, storage};
use crate::{
    events::{EventStore, NewEvent},
    routing::RoutingConfig,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use std::{collections::BTreeSet, io};

/// Explicit offline administrator input; no SMTP/API secret belongs in it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyInstallation {
    pub expected_revision: Option<i64>,
    pub policy: SitePolicy,
}

pub async fn install_policy(pool: &PgPool, input: &PolicyInstallation) -> Result<i64, sqlx::Error> {
    if !input.policy.valid() || input.expected_revision.is_some_and(|n| n <= 0) {
        return Err(storage());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='250ms'")
        .execute(&mut *tx)
        .await?;
    let owner: bool = sqlx::query_scalar("SELECT has_table_privilege(current_user,'core_console.options','INSERT') AND has_table_privilege(current_user,'core_console.options','UPDATE')")
        .fetch_one(&mut *tx).await?;
    if !owner {
        return Err(storage());
    }
    let previous: Option<i64> =
        sqlx::query_scalar("SELECT revision FROM core_console.options WHERE key=$1 FOR UPDATE")
            .bind(POLICY_KEY)
            .fetch_optional(&mut *tx)
            .await?;
    if previous != input.expected_revision {
        return Err(storage());
    }
    let revision = previous.unwrap_or(0).checked_add(1).ok_or_else(storage)?;
    let value = serde_json::to_string(&input.policy).map_err(|_| storage())?;
    EventStore::provision(
        &mut tx,
        CONSUMER,
        SERVICE,
        &EVENT_TYPES.map(str::to_owned),
        10_000,
    )
    .await
    .map_err(event_error)?;
    sqlx::query("INSERT INTO core_console.options(key,value,revision) VALUES($1,$2,$3) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value,revision=EXCLUDED.revision,updated_at=clock_timestamp()")
        .bind(POLICY_KEY).bind(value).bind(revision).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(revision)
}

/// The durable site policy, not a process-local environment flag, activates
/// every publisher. Mixed nodes cannot opt out while another node sends mail.
pub(crate) async fn load_policy(c: &mut PgConnection) -> Result<Option<SitePolicy>, sqlx::Error> {
    let installed: bool = sqlx::query_scalar("SELECT to_regclass('core_console.options') IS NOT NULL AND to_regclass('core_events.subscriptions') IS NOT NULL")
        .fetch_one(&mut *c).await?;
    if !installed {
        return Ok(None);
    }
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM core_console.options WHERE key=$1")
            .bind(POLICY_KEY)
            .fetch_optional(&mut *c)
            .await?;
    let Some(raw) = raw else { return Ok(None) };
    let policy: SitePolicy = serde_json::from_str(&raw).map_err(|_| storage())?;
    if !policy.valid() {
        return Err(storage());
    }
    let subscription = sqlx::query(
        "SELECT service_id,event_types FROM core_events.subscriptions WHERE consumer_id=$1",
    )
    .bind(CONSUMER)
    .fetch_optional(&mut *c)
    .await?
    .ok_or_else(storage)?;
    let expected: BTreeSet<_> = EVENT_TYPES.into_iter().map(str::to_owned).collect();
    if subscription.try_get::<String, _>("service_id")? != SERVICE
        || subscription
            .try_get::<Vec<String>, _>("event_types")?
            .into_iter()
            .collect::<BTreeSet<_>>()
            != expected
    {
        return Err(storage());
    }
    Ok(Some(policy))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Fact {
    pub revision: u32,
    pub kind: Kind,
    pub user_id: i64,
    pub evidence: Evidence,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Evidence {
    Balance {
        account_id: i64,
        journal_id: i64,
        before_credits: i64,
        after_credits: i64,
        threshold_credits: i64,
    },
    Routing {
        before_version: u64,
        after_version: u64,
        changed_channels: u64,
    },
    Models {
        before_version: u64,
        after_version: u64,
        added: u64,
        removed: u64,
    },
}
impl Fact {
    pub(super) fn valid(&self) -> bool {
        self.revision == 1
            && self.user_id > 0
            && match (&self.kind, &self.evidence) {
                (
                    Kind::Balance,
                    Evidence::Balance {
                        account_id,
                        journal_id,
                        before_credits,
                        after_credits,
                        threshold_credits,
                    },
                ) => {
                    *account_id > 0
                        && *journal_id > 0
                        && *threshold_credits > 0
                        && *before_credits >= *threshold_credits
                        && *after_credits < *threshold_credits
                }
                (
                    Kind::Channel,
                    Evidence::Routing {
                        before_version,
                        after_version,
                        changed_channels,
                    },
                ) => {
                    *before_version > 0
                        && *after_version > *before_version
                        && *after_version <= i64::MAX as u64
                        && *changed_channels > 0
                }
                (
                    Kind::Models,
                    Evidence::Models {
                        before_version,
                        after_version,
                        added,
                        removed,
                    },
                ) => {
                    *before_version > 0
                        && *after_version > *before_version
                        && *after_version <= i64::MAX as u64
                        && (*added > 0 || *removed > 0)
                }
                _ => false,
            }
    }
}

async fn publish(
    c: &mut PgConnection,
    key: &str,
    version: i64,
    fact: &Fact,
) -> Result<(), sqlx::Error> {
    if !fact.valid() {
        return Err(storage());
    }
    let bytes = serde_json::to_vec(fact).map_err(|_| storage())?;
    EventStore::publish_in_transaction(
        c,
        NewEvent {
            key,
            event_type: fact.kind.event_type(),
            schema_version: 1,
            resource_id: fact.user_id,
            resource_version: version,
            payload: &bytes,
        },
    )
    .await
    .map_err(event_error)?;
    Ok(())
}

fn advisory_key(account: i64) -> i64 {
    let mut digest = Sha256::new();
    digest.update(b"lmm.notifications.wallet.v1\0");
    digest.update(account.to_be_bytes());
    i64::from_be_bytes(
        digest.finalize()[..8]
            .try_into()
            .expect("fixed SHA256 prefix"),
    )
}

/// Called after the real ledger function, in its caller's still-open transaction.
/// No balance is written here. Contention returns immediately and the WHOLE
/// money transaction must roll back. Ordered advisory locks serialize observers
/// of otherwise disjoint wallet/reserved/debt bucket updates across all nodes.
pub(crate) async fn observe_ledger(
    c: &mut PgConnection,
    journal_id: i64,
) -> Result<(), sqlx::Error> {
    let Some(policy) = load_policy(c).await? else {
        return Ok(());
    };
    if !policy.quota_warning_enabled {
        return Ok(());
    }
    let isolation: String = sqlx::query_scalar("SELECT current_setting('transaction_isolation')")
        .fetch_one(&mut *c)
        .await?;
    if !matches!(isolation.as_str(), "read committed" | "serializable") {
        return Err(storage());
    }
    let fresh: bool = sqlx::query_scalar(
        "SELECT created_xid=pg_current_xact_id() FROM core_billing.ledger_journals WHERE id=$1",
    )
    .bind(journal_id)
    .fetch_one(&mut *c)
    .await?;
    // A historical idempotent replay is never relabelled with today's balance.
    if !fresh {
        return Ok(());
    }
    let changes = sqlx::query("SELECT a.owner_account_id AS account_id,sum(e.delta_units)::bigint AS delta FROM core_billing.ledger_entries e JOIN core_billing.ledger_accounts a ON a.id=e.ledger_account_id WHERE e.journal_id=$1 AND a.bucket IN ('wallet','reserved','debt') GROUP BY a.owner_account_id ORDER BY a.owner_account_id")
        .bind(journal_id).fetch_all(&mut *c).await?;
    for row in &changes {
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
            .bind(advisory_key(row.try_get("account_id")?))
            .fetch_one(&mut *c)
            .await?;
        if !locked {
            return Err(sqlx::Error::Io(io::Error::new(
                io::ErrorKind::WouldBlock,
                "notification account observer busy; retry the same operation",
            )));
        }
    }
    for row in changes {
        let account: i64 = row.try_get("account_id")?;
        let delta: i64 = row.try_get("delta")?;
        let profile = sqlx::query("SELECT u.id,u.platform_role,p.setting FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.personal_account_id=$1 AND u.active AND a.active")
            .bind(account).fetch_optional(&mut *c).await?;
        let Some(profile) = profile else { continue };
        let user: i64 = profile.try_get("id")?;
        let observed_key = format!("notify:observed:{journal_id}:{user}");
        let command = crate::events::CommandKey {
            service_id: SERVICE,
            actor_id: user,
            method: "notifications.balance.observed",
            idempotency_key: &observed_key,
        };
        let canonical = format!("v1:{journal_id}:{account}:{delta}");
        match crate::events::begin_command_in_transaction(c, &command, canonical.as_bytes())
            .await
            .map_err(event_error)?
        {
            crate::events::InboxOutcome::Replay(_) => continue,
            crate::events::InboxOutcome::New => {
                crate::events::complete_command_in_transaction(c, &command, b"observed")
                    .await
                    .map_err(event_error)?
            }
        }
        if delta >= 0 {
            continue;
        }
        let key = format!("notify:balance:{journal_id}:{user}");
        // Also handle an exact replay within the same transaction without
        // deriving a new balance from later journals in that transaction.
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_events.outbox WHERE event_key=$1)",
        )
        .bind(&key)
        .fetch_one(&mut *c)
        .await?;
        if exists {
            continue;
        }
        let prefs =
            Preferences::read(&profile.try_get::<String, _>("setting")?).map_err(|_| storage())?;
        if !prefs.enabled(
            Kind::Balance,
            &profile.try_get::<String, _>("platform_role")?,
        ) {
            continue;
        }
        let state = sqlx::query("SELECT sum(b.balance_units)::bigint AS total,count(*)::bigint AS buckets FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.owner_account_id=$1 AND a.bucket IN ('wallet','reserved','debt')")
            .bind(account).fetch_one(&mut *c).await?;
        if state.try_get::<i64, _>("buckets")? != 3 {
            return Err(storage());
        }
        let after: i64 = state.try_get("total")?;
        let threshold = prefs.threshold(&policy);
        let before = after.checked_sub(delta).ok_or_else(storage)?;
        if balance_edge(after, delta, threshold).is_none() {
            continue;
        }
        publish(
            c,
            &key,
            journal_id,
            &Fact {
                revision: 1,
                kind: Kind::Balance,
                user_id: user,
                evidence: Evidence::Balance {
                    account_id: account,
                    journal_id,
                    before_credits: before,
                    after_credits: after,
                    threshold_credits: threshold,
                },
            },
        )
        .await?;
    }
    Ok(())
}

fn model_bindings(config: &RoutingConfig) -> BTreeSet<(u64, String)> {
    config
        .routes
        .iter()
        .filter(|r| r.enabled)
        .flat_map(|r| {
            r.targets
                .iter()
                .filter(|t| t.enabled)
                .map(|t| (t.upstream_id, t.upstream_model.clone()))
        })
        .collect()
}

/// A durable publication fact, not a guessed health-probe or scheduled supplier
/// discovery result. Price/version-only releases do not emit a channel change.
pub(crate) async fn publish_routing_change(
    c: &mut PgConnection,
    before: &RoutingConfig,
    after: &RoutingConfig,
) -> Result<(), sqlx::Error> {
    if before.routes == after.routes
        && before.upstreams == after.upstreams
        && before.groups == after.groups
        && before.aliases == after.aliases
    {
        return Ok(());
    }
    let Some(policy) = load_policy(c).await? else {
        return Ok(());
    };
    if before.version == after.version {
        return Ok(());
    }
    if before.version == 0 || after.version <= before.version || after.version > i64::MAX as u64 {
        return Err(storage());
    }
    let ids: BTreeSet<u64> = before
        .upstreams
        .iter()
        .chain(&after.upstreams)
        .map(|u| u.id)
        .collect();
    let changed = ids
        .into_iter()
        .filter(|id| {
            before.upstreams.iter().find(|u| u.id == *id)
                != after.upstreams.iter().find(|u| u.id == *id)
                || before
                    .routes
                    .iter()
                    .filter(|r| r.targets.iter().any(|t| t.upstream_id == *id))
                    .map(|r| {
                        (
                            r.id,
                            &r.model,
                            &r.group,
                            r.enabled,
                            &r.speeds,
                            &r.failover,
                            r.targets
                                .iter()
                                .filter(|t| t.upstream_id == *id)
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
                    != after
                        .routes
                        .iter()
                        .filter(|r| r.targets.iter().any(|t| t.upstream_id == *id))
                        .map(|r| {
                            (
                                r.id,
                                &r.model,
                                &r.group,
                                r.enabled,
                                &r.speeds,
                                &r.failover,
                                r.targets
                                    .iter()
                                    .filter(|t| t.upstream_id == *id)
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>()
        })
        .count() as u64;
    let old_models = model_bindings(before);
    let new_models = model_bindings(after);
    let added = new_models.difference(&old_models).count() as u64;
    let removed = old_models.difference(&new_models).count() as u64;
    let admins = sqlx::query("SELECT u.id,u.platform_role,p.setting FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.active AND a.active AND u.platform_role IN ('admin','superadmin') ORDER BY u.id LIMIT 1001")
        .fetch_all(&mut *c).await?;
    if admins.len() > 1000 {
        return Err(storage());
    }
    for admin in admins {
        let user: i64 = admin.try_get("id")?;
        let role: String = admin.try_get("platform_role")?;
        let prefs =
            Preferences::read(&admin.try_get::<String, _>("setting")?).map_err(|_| storage())?;
        for kind in [Kind::Channel, Kind::Models] {
            if !policy.enabled(kind) || !prefs.enabled(kind, &role) {
                continue;
            }
            let evidence = match kind {
                Kind::Channel if changed > 0 => Evidence::Routing {
                    before_version: before.version,
                    after_version: after.version,
                    changed_channels: changed,
                },
                Kind::Models if added > 0 || removed > 0 => Evidence::Models {
                    before_version: before.version,
                    after_version: after.version,
                    added,
                    removed,
                },
                _ => continue,
            };
            let key = format!(
                "notify:{}:{}:{user}",
                if kind == Kind::Channel {
                    "channel"
                } else {
                    "models"
                },
                after.version
            );
            publish(
                c,
                &key,
                after.version as i64,
                &Fact {
                    revision: 1,
                    kind,
                    user_id: user,
                    evidence,
                },
            )
            .await?;
        }
    }
    Ok(())
}
