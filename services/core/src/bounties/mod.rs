//! Native bounty funds and immutable payout authority. Go only mirrors receipts.
//! Rules adapted from AGPL-3.0 repository revision 3948a0a model/open_source_bounty*.
use crate::{
    billing::{
        Credits, charging,
        ledger::{Action, Outcome, Request, Wallet},
    },
    identity::{self, CredentialKind, Principal},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row, types::Json};
pub mod http;
pub const SCHEMA: &str = include_str!("schema.sql");
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub enum Error {
    Invalid,
    Forbidden,
    Missing,
    Conflict,
    Storage,
}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Command {
    pub operation_id: String,
    pub action: String,
    pub project_id: String,
    pub challenge_id: String,
    pub dispute_id: String,
    pub confirmation: String,
    pub reward_credit: i64,
    pub reward_slots: i64,
    pub repository_url: String,
    pub credit: i64,
    pub approve: bool,
    pub issue_url: String,
    pub pull_request_url: String,
    pub github_handle: String,
    pub submission_note: String,
    pub review_note: String,
    pub rating_score: i64,
    pub rating_comment: String,
    pub reason: String,
    pub statement: String,
    pub resolution: String,
    pub resolve_action: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    #[serde(default)]
    pub project_revision: i64,
    #[serde(default)]
    pub published_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_credit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_basis_points: Option<i64>,
    pub operation_id: String,
    pub project_id: String,
    pub challenge_id: String,
    pub dispute_id: String,
    pub state: String,
    pub status: String,
    pub journal_ids: Vec<i64>,
    pub credit: i64,
    pub actor_user_id: i64,
    pub escrow_credit: i64,
    pub reward_credit: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub confirmation: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expires: i64,
}
fn is_zero(v: &i64) -> bool {
    *v == 0
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub confirmation: String,
    pub expires: i64,
    pub credit: i64,
    pub fee_credit: i64,
    pub reward_credit: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Slot {
    hold: i64,
    paid: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Claim {
    id: String,
    user: i64,
    account: i64,
    state: String,
    rejected_at: i64,
    dispute: String,
    resolved: bool,
    case_users: Vec<i64>,
    issue: String,
    pr: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Project {
    #[serde(default)]
    revision: i64,
    #[serde(default)]
    published_at: i64,
    owner: i64,
    payer: i64,
    status: String,
    repository: String,
    gross: i64,
    net: i64,
    fee: i64,
    #[serde(default)]
    fee_bps: Option<i64>,
    consumption: String,
    publication_operation: String,
    slots: Vec<Slot>,
    claims: Vec<Claim>,
}
#[derive(Clone)]
pub struct BountyStore {
    pool: PgPool,
}
fn hash<T: Serialize>(x: &T) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(x).map_err(|_| Error::Invalid)?)
    ))
}
fn clean(c: &Command) -> Command {
    let mut c = c.clone();
    c.confirmation.clear();
    c
}
fn repository(raw: &str) -> Result<String> {
    let u = reqwest::Url::parse(raw).map_err(|_| Error::Invalid)?;
    if u.scheme() != "https"
        || u.host_str() != Some("github.com")
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(Error::Invalid);
    }
    let path = u.path().trim_end_matches('/').trim_end_matches(".git");
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
    {
        return Err(Error::Invalid);
    }
    Ok(format!("https://github.com/{}/{}", parts[0], parts[1]))
}
fn name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}
/// Legacy charges ceil(gross*bps/10000) per slot, never rounded USD floats.
pub fn publication(gross: i64, slots: i64, bps: i64) -> Result<(i64, i64, i64)> {
    if gross <= 0 || !(1..=100).contains(&slots) || !(0..=9999).contains(&bps) {
        return Err(Error::Invalid);
    }
    let fee = i64::try_from((i128::from(gross) * i128::from(bps) + 9999) / 10000)
        .map_err(|_| Error::Invalid)?;
    if fee >= gross {
        return Err(Error::Invalid);
    }
    Ok((
        gross.checked_mul(slots).ok_or(Error::Invalid)?,
        fee.checked_mul(slots).ok_or(Error::Invalid)?,
        gross - fee,
    ))
}
async fn actor(c: &mut PgConnection, s: &str) -> Result<Principal> {
    let a = identity::authorize_in_transaction(c, s)
        .await
        .map_err(|_| Error::Forbidden)?;
    if a.credential_kind != CredentialKind::Session
        || a.owner.kind != crate::accounts::AccountKind::Personal
        || a.owner.id != a.user_id
    {
        return Err(Error::Forbidden);
    }
    Ok(a)
}
// Private bounty admission uses Core's live developer authority. Existing holds
// retain their settlement, appeal, return and recovery paths after downgrade.
fn requires_developer(action: &str) -> bool {
    !matches!(
        action,
        "close"
            | "approve"
            | "reject"
            | "withdraw"
            | "cancel"
            | "submit"
            | "open_dispute"
            | "resolve_dispute"
    )
}
async fn developer(c: &mut PgConnection, a: &Principal, secret: &str) -> Result<()> {
    identity::require_bounty_developer_in_transaction(c, a, secret)
        .await
        .map_err(|error| match error {
            identity::IdentityError::Storage | identity::IdentityError::Conflict => Error::Storage,
            _ => Error::Forbidden,
        })
}
// Static administrator admission reuses Core's current Session, MFA, role and
// complete imported-policy gate. This alone grants no spend: the frozen dispute
// state and explicit browser confirmation are checked separately below.
async fn administrator(c: &mut PgConnection, secret: &str) -> Result<bool> {
    match identity::authorize_admin_read_snapshot(c, secret).await {
        Ok(_) => Ok(true),
        Err(identity::admin_users::AdminUsersError::Unavailable) => Err(Error::Storage),
        Err(_) => Ok(false),
    }
}
async fn now(c: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(c)
            .await?,
    )
}
async fn post(
    c: &mut PgConnection,
    cmd: &Command,
    actor: i64,
    phase: &str,
    action: Action,
) -> Result<i64> {
    let r = Request {
        scope: "core-bounty".into(),
        operation_key: hash(&[&cmd.operation_id, phase])?,
        actor_user_id: Some(actor),
        reason: format!("Bounty {} {}", cmd.project_id, phase),
        action,
    };
    r.validate().map_err(|_| Error::Invalid)?;
    let Json(o): Json<Outcome> = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(r))
        .fetch_one(c)
        .await?;
    match o {
        Outcome::Posted { journal_id, .. } => Ok(journal_id),
        _ => Err(Error::Conflict),
    }
}
async fn wallet(c: &mut PgConnection, id: i64) -> Result<()> {
    let _: Json<Wallet> = sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
        .bind(id)
        .fetch_one(c)
        .await?;
    Ok(())
}
async fn budget(
    c: &mut PgConnection,
    cmd: &Command,
    a: &Principal,
    amount: i64,
    t: i64,
) -> Result<()> {
    identity::reject_marked_new_action(c, a.user_id, identity::NewResourceAction::CommercePayment)
        .await
        .map_err(|_| Error::Forbidden)?;
    let id = format!("bounty:{}", hash(&cmd.operation_id)?);
    sqlx::query("INSERT INTO core_charging.external_consumption(id,domain,actor_user_id,key_id,payer_account_id,authorized_at,amount_credits) VALUES($1,'bounty',$2,$3,$4,$5,$6)").bind(&id).bind(a.user_id).bind(a.credential_id).bind(a.owner_account_id).bind(t).bind(amount).execute(&mut *c).await?;
    charging::external_budgets(c, &id)
        .await
        .map_err(|_| Error::Forbidden)
}
fn valid(cmd: &Command) -> Result<()> {
    if !name(&cmd.operation_id)
        || !name(&cmd.project_id)
        || (!cmd.challenge_id.is_empty() && !name(&cmd.challenge_id))
        || (!cmd.dispute_id.is_empty() && !name(&cmd.dispute_id))
        || serde_json::to_vec(cmd).map_err(|_| Error::Invalid)?.len() > 24000
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
impl BountyStore {
    /// Side-effect-free startup proof; installation remains an offline owner action.
    pub async fn check_ready(&self) -> Result<()> {
        crate::runtime_database::check(
            &self.pool,
            crate::runtime_database::RequiredCapabilities {
                bounties: true,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| Error::Storage)
    }
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn preview(&self, secret: &str, cmd: &Command) -> Result<Preview> {
        valid(cmd)?;
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, secret).await?;
        if requires_developer(&cmd.action) {
            developer(&mut tx, &a, secret).await?;
        }
        let t = now(&mut tx).await?;
        let mut policy_basis = None;
        let (credit, fee, net) = if cmd.action == "publish" {
            let r = sqlx::query(
                "SELECT p.fee_bps,p.fee_account_id FROM core_bounties.policy p JOIN core_identity.users u ON u.personal_account_id=p.fee_account_id JOIN core_identity.accounts a ON a.id=p.fee_account_id WHERE p.id AND u.active AND a.active AND u.platform_role='superadmin' FOR SHARE OF u,a",
            )
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Missing)?;
            let bps: i64 = r.try_get("fee_bps")?;
            policy_basis = Some((bps, r.try_get::<i64, _>("fee_account_id")?));
            publication(cmd.reward_credit, cmd.reward_slots, bps)?
        } else {
            (cmd.credit, 0, 0)
        };
        let req = clean(cmd);
        let confirmation = hash(&(a.user_id, a.credential_id, &req, t))?;
        sqlx::query("INSERT INTO core_bounties.confirmations(id,actor,credential,request,expires,policy) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING").bind(&confirmation).bind(a.user_id).bind(a.credential_id).bind(Json(req)).bind(t+300).bind(Json(policy_basis)).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Preview {
            confirmation,
            expires: t + 300,
            credit,
            fee_credit: fee,
            reward_credit: net,
        })
    }
    pub async fn execute(&self, secret: &str, cmd: &Command) -> Result<Receipt> {
        valid(cmd)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await?;
        let a = actor(&mut tx, secret).await?;
        let t = now(&mut tx).await?;
        let req = clean(cmd);
        // Locks serialize identical operation keys before any side effect or state read.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,742199))")
            .bind(&cmd.operation_id)
            .execute(&mut *tx)
            .await?;
        if let Some(r) =
            sqlx::query("SELECT actor,request,receipt FROM core_bounties.operations WHERE id=$1")
                .bind(&cmd.operation_id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let Json(old): Json<Command> = r.try_get("request")?;
            if old != req || r.try_get::<i64, _>("actor")? != a.user_id {
                return Err(Error::Conflict);
            }
            let Json(receipt) = r.try_get("receipt")?;
            tx.commit().await?;
            return Ok(receipt);
        }
        if requires_developer(&cmd.action) {
            developer(&mut tx, &a, secret).await?;
        }
        let r=sqlx::query("SELECT actor,credential,request,expires,policy,approved FROM core_bounties.confirmations WHERE id=$1 FOR UPDATE").bind(&cmd.confirmation).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        let Json(confirmed): Json<Command> = r.try_get("request")?;
        if !r.try_get::<bool, _>("approved")?
            || confirmed != req
            || r.try_get::<i64, _>("actor")? != a.user_id
            || r.try_get::<i64, _>("credential")? != a.credential_id
            || r.try_get::<i64, _>("expires")? < t
        {
            return Err(Error::Forbidden);
        }
        let Json(policy_basis): Json<Option<(i64, i64)>> = r.try_get("policy")?;
        if cmd.action == "delete_draft" {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM core_bounties.projects WHERE id=$1)",
            )
            .bind(&cmd.project_id)
            .fetch_one(&mut *tx)
            .await?;
            if exists {
                return Err(Error::Conflict);
            }
            let receipt = Receipt {
                project_revision: 0,
                published_at: 0,
                remaining_credit: None,
                fee_basis_points: None,
                confirmation: String::new(),
                expires: 0,
                operation_id: cmd.operation_id.clone(),
                project_id: cmd.project_id.clone(),
                challenge_id: String::new(),
                dispute_id: String::new(),
                state: "deleted".into(),
                status: "deleted".into(),
                journal_ids: Vec::new(),
                credit: 0,
                actor_user_id: a.user_id,
                escrow_credit: 0,
                reward_credit: 0,
            };
            sqlx::query("INSERT INTO core_bounties.operations(id,actor,request,receipt) VALUES($1,$2,$3,$4)").bind(&cmd.operation_id).bind(a.user_id).bind(Json(req)).bind(Json(&receipt)).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(receipt);
        }
        let mut journals = Vec::new();
        let mut credit = 0;
        let mut p = if cmd.action == "publish" {
            if sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM core_bounties.projects WHERE id=$1)",
            )
            .bind(&cmd.project_id)
            .fetch_one(&mut *tx)
            .await?
            {
                return Err(Error::Conflict);
            }
            let normalized_repository = repository(&cmd.repository_url)?;
            let policy = sqlx::query(
                "SELECT p.fee_bps,p.fee_account_id FROM core_bounties.policy p JOIN core_identity.users u ON u.personal_account_id=p.fee_account_id JOIN core_identity.accounts a ON a.id=p.fee_account_id WHERE p.id AND u.active AND a.active AND u.platform_role='superadmin' FOR SHARE OF u,a",
            )
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Missing)?;
            if policy_basis
                != Some((
                    policy.try_get("fee_bps")?,
                    policy.try_get("fee_account_id")?,
                ))
            {
                return Err(Error::Conflict);
            }
            let (total, fee, net) = publication(
                cmd.reward_credit,
                cmd.reward_slots,
                policy.try_get("fee_bps")?,
            )?;
            budget(&mut tx, cmd, &a, total, t).await?;
            wallet(&mut tx, a.owner_account_id).await?;
            let fee_account: i64 = policy.try_get("fee_account_id")?;
            wallet(&mut tx, fee_account).await?;
            let mut slots = Vec::new();
            for n in 0..cmd.reward_slots {
                let hold = post(
                    &mut tx,
                    cmd,
                    a.user_id,
                    &format!("slot-{n}"),
                    Action::Reserve {
                        account_id: a.owner_account_id,
                        amount_units: Credits::try_from(net).map_err(|_| Error::Invalid)?,
                    },
                )
                .await?;
                journals.push(hold);
                slots.push(Slot { hold, paid: false })
            }
            if fee > 0 && fee_account != a.owner_account_id {
                journals.push(
                    post(
                        &mut tx,
                        cmd,
                        a.user_id,
                        "fee",
                        Action::Transfer {
                            from_account_id: a.owner_account_id,
                            to_account_id: fee_account,
                            amount_units: Credits::try_from(fee).map_err(|_| Error::Invalid)?,
                        },
                    )
                    .await?,
                )
            }
            credit = total;
            Project {
                revision: 0,
                published_at: t,
                owner: a.user_id,
                payer: a.owner_account_id,
                status: "published".into(),
                repository: normalized_repository,
                gross: cmd.reward_credit,
                net,
                fee,
                fee_bps: Some(policy.try_get("fee_bps")?),
                consumption: format!("bounty:{}", hash(&cmd.operation_id)?),
                publication_operation: cmd.operation_id.clone(),
                slots,
                claims: Vec::new(),
            }
        } else {
            let Json(p): Json<Project> = sqlx::query_scalar(
                "SELECT body FROM core_bounties.projects WHERE id=$1 FOR UPDATE",
            )
            .bind(&cmd.project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Missing)?;
            p
        };
        let mut state = p.status.clone();
        if cmd.action != "publish" {
            transition(
                &mut tx,
                cmd,
                &a,
                secret,
                t,
                &mut p,
                &mut journals,
                &mut credit,
                &mut state,
            )
            .await?
        }
        p.revision = p.revision.checked_add(1).ok_or(Error::Conflict)?;
        let escrow = p
            .slots
            .iter()
            .filter(|s| !s.paid)
            .try_fold(0i64, |v, _| v.checked_add(p.net))
            .ok_or(Error::Invalid)?;
        let remaining_credit = if cmd.action == "tip" {
            Some(sqlx::query_scalar("SELECT (coalesce(sum(b.balance_units) FILTER(WHERE a.bucket='wallet'),0)+coalesce(sum(b.balance_units) FILTER(WHERE a.bucket='debt'),0))::bigint FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.owner_account_id=$1 AND a.bucket IN ('wallet','debt')").bind(p.payer).fetch_one(&mut *tx).await?)
        } else {
            None
        };
        let receipt = Receipt {
            remaining_credit,
            fee_basis_points: p.fee_bps,
            project_revision: p.revision,
            published_at: p.published_at,
            confirmation: String::new(),
            expires: 0,
            operation_id: cmd.operation_id.clone(),
            project_id: cmd.project_id.clone(),
            challenge_id: cmd.challenge_id.clone(),
            dispute_id: cmd.dispute_id.clone(),
            state,
            status: p.status.clone(),
            journal_ids: journals,
            credit,
            actor_user_id: a.user_id,
            escrow_credit: escrow,
            reward_credit: p.net,
        };
        if cmd.action == "publish" {
            sqlx::query("INSERT INTO core_bounties.projects(id,body) VALUES($1,$2)")
                .bind(&cmd.project_id)
                .bind(Json(p))
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE core_bounties.projects SET body=$2 WHERE id=$1")
                .bind(&cmd.project_id)
                .bind(Json(p))
                .execute(&mut *tx)
                .await?;
        }

        sqlx::query(
            "INSERT INTO core_bounties.operations(id,actor,request,receipt) VALUES($1,$2,$3,$4)",
        )
        .bind(&cmd.operation_id)
        .bind(a.user_id)
        .bind(Json(req))
        .bind(Json(&receipt))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(receipt)
    }
}
fn protected(c: &Claim, t: i64) -> bool {
    !c.dispute.is_empty()
        || matches!(c.state.as_str(), "accepted" | "submitted" | "approved")
        || (c.state == "rejected" && !c.resolved && c.rejected_at > t - 604800)
}
fn evidence(url: &str, repo: &str, kind: &str) -> bool {
    url.strip_prefix(repo.trim_end_matches('/'))
        .is_some_and(|s| {
            s.strip_prefix(kind)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
}
// Connection, command, actor, secret and mutable project fields stay separate.
#[allow(clippy::too_many_arguments)]
async fn transition(
    c: &mut PgConnection,
    cmd: &Command,
    a: &Principal,
    secret: &str,
    t: i64,
    p: &mut Project,
    j: &mut Vec<i64>,
    credit: &mut i64,
    state: &mut String,
) -> Result<()> {
    let owner = a.user_id == p.owner;
    if matches!(cmd.action.as_str(), "accept" | "approve")
        || (cmd.action == "resolve_dispute" && cmd.resolve_action == "pay")
    {
        identity::reject_marked_new_action(
            c,
            a.user_id,
            identity::NewResourceAction::CommercePayment,
        )
        .await
        .map_err(|_| Error::Forbidden)?;
    }
    match cmd.action.as_str() {
        "pause" | "resume" => {
            if !owner || !matches!(p.status.as_str(), "published" | "paused") {
                return Err(Error::Forbidden);
            }
            p.status = if cmd.action == "pause" {
                "paused"
            } else {
                "published"
            }
            .into();
            *state = p.status.clone();
        }
        "accept" => {
            if p.status != "published"
                || owner
                || !name(&cmd.challenge_id)
                || cmd.github_handle.is_empty()
                || cmd.github_handle.len() > 39
                || !cmd
                    .github_handle
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(Error::Invalid);
            }
            if p.claims
                .iter()
                .any(|x| x.id == cmd.challenge_id || (x.user == a.user_id && protected(x, t)))
                || p.claims.iter().filter(|x| protected(x, t)).count() >= p.slots.len()
            {
                return Err(Error::Conflict);
            }
            p.claims.push(Claim {
                id: cmd.challenge_id.clone(),
                user: a.user_id,
                account: a.owner_account_id,
                state: "accepted".into(),
                rejected_at: 0,
                dispute: String::new(),
                resolved: false,
                case_users: Vec::new(),
                issue: String::new(),
                pr: String::new(),
            });
            *state = "accepted".into();
        }
        "close" => {
            if !owner
                || !matches!(p.status.as_str(), "published" | "paused")
                || p.claims.iter().any(|x| {
                    !x.dispute.is_empty()
                        || matches!(x.state.as_str(), "accepted" | "submitted")
                        || (x.state == "rejected" && !x.resolved && x.rejected_at > t - 604800)
                })
            {
                return Err(Error::Conflict);
            }
            for (n, s) in p.slots.iter_mut().enumerate() {
                if !s.paid {
                    j.push(
                        post(
                            c,
                            cmd,
                            a.user_id,
                            &format!("release-{n}"),
                            Action::Release {
                                reservation_journal_id: s.hold,
                            },
                        )
                        .await?,
                    );
                    s.paid = true;
                    *credit = credit.checked_add(p.net).ok_or(Error::Invalid)?
                }
            }
            // Return unused commitment to original budget consumption; fees remain charged.

            sqlx::query("UPDATE core_charging.external_consumption SET refunded=refunded+$2 WHERE id=$1 AND domain='bounty' AND payer_account_id=$3").bind(&p.consumption).bind(*credit).bind(p.payer).execute(&mut *c).await?;
            p.status = "closed".into();
            *state = p.status.clone();
        }
        "publish" => return Err(Error::Conflict),
        _ => {
            let n = p
                .claims
                .iter()
                .position(|x| x.id == cmd.challenge_id)
                .ok_or(Error::Missing)?;
            if p.status == "closed" && cmd.action != "tip" {
                return Err(Error::Conflict);
            }
            let x = &mut p.claims[n];
            match cmd.action.as_str() {
                "submit" => {
                    if x.user != a.user_id
                        || !matches!(x.state.as_str(), "accepted" | "submitted")
                        || (!evidence(&cmd.issue_url, &p.repository, "/issues/")
                            && !evidence(&cmd.pull_request_url, &p.repository, "/pull/"))
                    {
                        return Err(Error::Forbidden);
                    }
                    x.state = "submitted".into();
                    x.issue = cmd.issue_url.clone();
                    x.pr = cmd.pull_request_url.clone();
                }
                "withdraw" | "cancel" => {
                    if !x.dispute.is_empty()
                        || (cmd.action == "cancel" && x.state != "accepted")
                        || (cmd.action == "withdraw"
                            && !matches!(x.state.as_str(), "accepted" | "submitted"))
                        || (cmd.action == "withdraw" && x.user != a.user_id)
                        || (cmd.action == "cancel" && !owner)
                    {
                        return Err(Error::Forbidden);
                    }
                    x.state = if cmd.action == "withdraw" {
                        "withdrawn"
                    } else {
                        "cancelled"
                    }
                    .into();
                }
                "reject" => {
                    if !owner || x.state != "submitted" || !x.dispute.is_empty() {
                        return Err(Error::Forbidden);
                    }
                    x.state = "rejected".into();
                    x.rejected_at = t;
                }
                "approve" => {
                    if !owner || x.state != "submitted" || !x.dispute.is_empty() {
                        return Err(Error::Forbidden);
                    }
                    pay(c, cmd, a.user_id, p, n, j, credit).await?;
                }
                "tip" => {
                    if !owner
                        || matches!(x.state.as_str(), "withdrawn" | "cancelled")
                        || cmd.credit <= 0
                    {
                        return Err(Error::Forbidden);
                    }
                    let recipient = x.account;
                    budget(c, cmd, a, cmd.credit, t).await?;
                    wallet(c, recipient).await?;
                    j.push(
                        post(
                            c,
                            cmd,
                            a.user_id,
                            "tip",
                            Action::Transfer {
                                from_account_id: p.payer,
                                to_account_id: recipient,
                                amount_units: Credits::try_from(cmd.credit)
                                    .map_err(|_| Error::Invalid)?,
                            },
                        )
                        .await?,
                    );
                    *credit = cmd.credit;
                }
                "open_dispute" => {
                    if (!owner && a.user_id != x.user)
                        || !name(&cmd.dispute_id)
                        || !x.dispute.is_empty()
                        || x.case_users.contains(&a.user_id)
                        || matches!(x.state.as_str(), "withdrawn" | "cancelled")
                        || (x.state == "rejected" && x.rejected_at <= t - 604800)
                        || cmd.statement.len() < 20
                        || cmd.statement.len() > 5000
                        || ![
                            "merged_but_unpaid",
                            "requirements_met_but_rejected",
                            "misleading_requirements",
                            "abusive_conduct",
                            "other",
                        ]
                        .contains(&cmd.reason.as_str())
                    {
                        return Err(Error::Forbidden);
                    }
                    x.dispute = cmd.dispute_id.clone();
                    x.case_users.push(a.user_id);
                    x.resolved = false;
                    *state = "open".into();
                    return Ok(());
                }
                "resolve_dispute" => {
                    if !administrator(c, secret).await?
                        || x.dispute != cmd.dispute_id
                        || x.dispute.is_empty()
                        || cmd.resolution.len() < 20
                        || cmd.resolution.len() > 5000
                    {
                        return Err(Error::Forbidden);
                    }
                    match cmd.resolve_action.as_str() {
                        "pay" => {
                            if !matches!(x.state.as_str(), "submitted" | "rejected") {
                                return Err(Error::Conflict);
                            }
                            pay(c, cmd, a.user_id, p, n, j, credit).await?;
                            *state = "resolved_paid".into();
                        }
                        "deny" => {
                            *state = "resolved_denied".into();
                        }
                        _ => return Err(Error::Invalid),
                    }
                    p.claims[n].dispute.clear();
                    p.claims[n].resolved = true;
                    return Ok(());
                }
                "rate_owner" => {
                    if x.user != a.user_id
                        || x.state != "approved"
                        || !(1..=5).contains(&cmd.rating_score)
                        || cmd.rating_comment.len() > 1000
                    {
                        return Err(Error::Forbidden);
                    }
                }
                _ => return Err(Error::Invalid),
            }
            *state = p.claims[n].state.clone();
        }
    }
    Ok(())
}
async fn pay(
    c: &mut PgConnection,
    cmd: &Command,
    actor: i64,
    p: &mut Project,
    n: usize,
    j: &mut Vec<i64>,
    credit: &mut i64,
) -> Result<()> {
    let slot = p
        .slots
        .iter_mut()
        .find(|s| !s.paid)
        .ok_or(Error::Conflict)?;
    let recipient = p.claims[n].account;
    wallet(c, recipient).await?;
    j.push(
        post(
            c,
            cmd,
            actor,
            "payout",
            Action::CommerceCapture {
                reservation_journal_id: slot.hold,
                destination_account_id: recipient,
                amount_units: Credits::try_from(p.net).map_err(|_| Error::Invalid)?,
            },
        )
        .await?,
    );
    slot.paid = true;
    p.claims[n].state = "approved".into();
    *credit = p.net;
    if p.slots.iter().all(|s| s.paid) {
        p.status = "completed".into()
    }
    Ok(())
}
#[derive(Serialize)]
pub struct Actor {
    pub user_id: i64,
    pub admin: bool,
}
#[derive(Serialize)]
pub struct FeeConfig {
    pub enabled: bool,
    pub code: String,
    pub rate_basis_points: i64,
    pub rate_percent: String,
    pub limits: FeeLimits,
}
#[derive(Serialize)]
pub struct FeeLimits {
    pub reward_slots_min: i64,
    pub reward_slots_max: i64,
    pub tip_credit_min: i64,
}
impl BountyStore {
    pub async fn session_authorize(&self, secret: &str) -> Result<Actor> {
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, secret).await?;
        let admin = administrator(&mut tx, secret).await?;
        tx.commit().await?;
        Ok(Actor {
            user_id: a.user_id,
            admin,
        })
    }
    pub async fn fee_config(&self) -> Result<FeeConfig> {
        let configured: Option<i64> =
            sqlx::query_scalar("SELECT fee_bps FROM core_bounties.policy WHERE id")
                .fetch_optional(&self.pool)
                .await?;
        let bps = configured.unwrap_or(0);
        Ok(FeeConfig {
            enabled: configured.is_some(),
            code: if configured.is_some() {
                "configured"
            } else {
                "policy_not_configured"
            }
            .into(),
            rate_basis_points: bps,
            rate_percent: format!("{}.{:02}", bps / 100, bps % 100),
            limits: FeeLimits {
                reward_slots_min: 1,
                reward_slots_max: 100,
                tip_credit_min: 1,
            },
        })
    }

    pub async fn authorize(&self, secret: &str) -> Result<Actor> {
        let mut tx = self.pool.begin().await?;
        let a = actor(&mut tx, secret).await?;
        developer(&mut tx, &a, secret).await?;
        let admin = administrator(&mut tx, secret).await?;
        tx.commit().await?;
        Ok(Actor {
            user_id: a.user_id,
            admin,
        })
    }
}

impl BountyStore {
    /// Durable recovery only. Never confirms, executes, posts or generates a new key.
    pub async fn query(&self, secret: &str, operation_id: &str) -> Result<Receipt> {
        self.query_inner(secret, operation_id, None).await
    }
    /// Projection recovery must prove every original argument, including evidence
    /// and ratings, before Go is allowed to rebuild its business record.
    pub async fn query_for_command(&self, secret: &str, expected: &Command) -> Result<Receipt> {
        valid(expected)?;
        self.query_inner(secret, &expected.operation_id, Some(expected))
            .await
    }
    async fn query_inner(
        &self,
        secret: &str,
        operation_id: &str,
        expected: Option<&Command>,
    ) -> Result<Receipt> {
        if !name(operation_id) {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let a = identity::authenticate_readonly_in_transaction(&mut tx, secret)
            .await
            .map_err(|_| Error::Forbidden)?;
        if a.credential_kind != CredentialKind::Session
            || a.owner.kind != crate::accounts::AccountKind::Personal
            || a.owner.id != a.user_id
        {
            return Err(Error::Forbidden);
        }
        if let Some(row) = sqlx::query(
            "SELECT request,receipt FROM core_bounties.operations WHERE id=$1 AND actor=$2",
        )
        .bind(operation_id)
        .bind(a.user_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let Json(saved): Json<Command> = row.try_get("request")?;
            if expected.is_some_and(|v| clean(v) != saved) {
                return Err(Error::Conflict);
            }
            let Json(receipt): Json<Receipt> = row.try_get("receipt")?;
            tx.commit().await?;
            return Ok(receipt);
        }
        let variants:i64=sqlx::query_scalar("SELECT count(DISTINCT request) FROM core_bounties.confirmations WHERE actor=$1 AND request->>'operation_id'=$2").bind(a.user_id).bind(operation_id).fetch_one(&mut *tx).await?;
        if variants == 0 {
            return Err(Error::Missing);
        }
        if variants != 1 {
            return Err(Error::Conflict);
        }
        let row=sqlx::query("SELECT id,request,policy,approved,expires FROM core_bounties.confirmations WHERE actor=$1 AND request->>'operation_id'=$2 ORDER BY (approved AND expires>=extract(epoch FROM clock_timestamp())::bigint) DESC,expires DESC,id DESC LIMIT 1").bind(a.user_id).bind(operation_id).fetch_one(&mut *tx).await?;
        let r = &row;
        let Json(cmd): Json<Command> = r.try_get("request")?;
        if expected.is_some_and(|v| clean(v) != cmd) {
            return Err(Error::Conflict);
        }
        let t = now(&mut tx).await?;
        let expires: i64 = r.try_get("expires")?;
        let approved: bool = r.try_get("approved")?;
        let state = if expires < t {
            "confirmation_expired"
        } else if approved {
            "confirmed"
        } else {
            "awaiting_confirmation"
        };
        let Json(policy): Json<Option<(i64, i64)>> = r.try_get("policy")?;
        let (credit, reward_credit, escrow_credit) = if cmd.action == "publish" {
            let (bps, _) = policy.ok_or(Error::Storage)?;
            let (total, _, net) = publication(cmd.reward_credit, cmd.reward_slots, bps)?;
            (
                total,
                net,
                net.checked_mul(cmd.reward_slots).ok_or(Error::Invalid)?,
            )
        } else {
            (cmd.credit, 0, 0)
        };
        let receipt = Receipt {
            project_revision: 0,
            published_at: 0,
            remaining_credit: None,
            fee_basis_points: policy.map(|p| p.0),
            operation_id: operation_id.into(),
            project_id: cmd.project_id,
            challenge_id: cmd.challenge_id,
            dispute_id: cmd.dispute_id,
            state: state.into(),
            status: state.into(),
            journal_ids: Vec::new(),
            credit,
            actor_user_id: a.user_id,
            escrow_credit,
            reward_credit,
            confirmation: r.try_get("id")?,
            expires,
        };
        tx.commit().await?;
        Ok(receipt)
    }
}
