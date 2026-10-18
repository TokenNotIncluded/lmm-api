//! Assistant delegation invariants, evaluated using Core-owned state.
//!
//! This module neither authenticates a bearer nor issues spending authority.
//! The host must verify the original Session and load/lock the grant, current
//! policy, conversation ownership, operation and budget in the SAME charging
//! transaction. None of those inputs may be deserialized from Go as authority.
//! There is deliberately no API-key/root-key adapter, signature scheme, network
//! endpoint, in-memory grant store or fallback when durable authority is absent.
use std::collections::BTreeSet;

pub const VERSION: u16 = 1;
pub const MAX_CREDITS: i64 = 9_007_199_254_740_991;
pub const MAX_TTL_SECONDS: i64 = 900;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Tenant {
    Personal {
        user_id: i64,
        account_id: i64,
    },
    Team {
        team_id: i64,
        account_id: i64,
        membership_version: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Funding {
    Personal {
        account_id: i64,
    },
    Team {
        team_id: i64,
        account_id: i64,
    },
    /// Only a persisted Core policy may construct this variant. A host token,
    /// client-selected payer or root API key is never a sponsorship policy.
    Sponsored {
        account_id: i64,
        policy_id: String,
        policy_version: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ToolScope {
    pub name: String,
    pub version: String,
    pub price_version: String,
    pub schema_sha256: [u8; 32],
}

/// An immutable receipt from Core's verified Session and persisted policy.
/// This is a domain input, NOT a wire DTO or evidence of authentication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Authority {
    pub actor_user_id: i64,
    pub session_id: i64,
    pub auth_version: i64,
    pub tenant: Tenant,
    pub conversation_id: String,
    pub conversation_owner_user_id: i64,
    pub conversation_owner_account_id: i64,
    pub turn_id: String,
    pub funding: Funding,
    pub model: String,
    pub group: String,
    pub price_version: String,
    pub tools: Vec<ToolScope>,
    pub budget_credits: i64,
    pub max_model_calls: u8,
    pub revocation_version: i64,
}

/// An opaque identifier is a lookup key; possession never authenticates Go/user.
/// No Deserialize implementation is provided for a trusted grant.
#[derive(Clone)]
pub struct Grant {
    id: String,
    authority: Authority,
    issued_at: i64,
    expires_at: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    Conflict,
    BudgetExceeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Reserved,
    Dispatched,
    Streaming,
    Settled,
    Released,
    Unknown,
}

/// Go supplies only identifiers and a payload digest. Core recomputes that
/// digest from the bounded, validated payload before calling `check_attempt`.
#[derive(Clone, Debug)]
pub struct Invocation {
    pub version: u16,
    pub delegation_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub round: u8,
    pub operation_key: String,
    pub payload_sha256: [u8; 32],
}

/// Persisted immutable operation binding, read under the grant/operation lock.
#[derive(Clone, Debug)]
pub struct ExistingOperation {
    pub delegation_id: String,
    pub operation_key: String,
    pub payload_sha256: [u8; 32],
    pub status: Status,
}

/// Same-transaction grant-wide totals. Holds include UNKNOWN operations and
/// every round of the turn; refreshing/reissuing access cannot reset them.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub held_credits: i64,
    pub settled_credits: i64,
    pub reserve_credits: i64,
}

/// A single locked database view, obtained after current Session verification.
pub struct CurrentState<'a> {
    pub authority: &'a Authority,
    pub now: i64,
    pub revoked: bool,
    /// Derived from durable settled preceding rounds, never from the caller.
    pub next_round: u8,
    pub budget: Budget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    /// The caller still must atomically persist the hold + operation and claim
    /// its worker before any dispatch. This value alone permits no HTTP call.
    ReserveNew,
    ReplayTerminal(Status),
    /// Inspect/reconcile the ORIGINAL operation. Never dispatch another attempt
    /// after a timeout, unknown COMMIT, process restart or partial output.
    InspectOriginal(Status),
}

fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn grant_id(value: &str) -> bool {
    value.strip_prefix("lmm_ad_").is_some_and(|id| {
        id.len() == 64
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn valid_authority(a: &Authority) -> bool {
    if [
        a.actor_user_id,
        a.session_id,
        a.auth_version,
        a.conversation_owner_user_id,
        a.conversation_owner_account_id,
        a.revocation_version,
    ]
    .iter()
    .any(|v| *v <= 0)
        || a.actor_user_id != a.conversation_owner_user_id
        || !(1..=MAX_CREDITS).contains(&a.budget_credits)
        || !(1..=4).contains(&a.max_model_calls)
        || [
            &a.conversation_id,
            &a.turn_id,
            &a.model,
            &a.group,
            &a.price_version,
        ]
        .iter()
        .any(|v| !label(v))
        || a.tools.len() > 64
        || a.turn_id.len() > 124
    {
        return false;
    }
    let account = match a.tenant {
        Tenant::Personal {
            user_id,
            account_id,
        } if user_id == a.actor_user_id && account_id > 0 => account_id,
        Tenant::Team {
            team_id,
            account_id,
            membership_version,
        } if team_id > 0 && account_id > 0 && membership_version > 0 => account_id,
        _ => return false,
    };
    if account != a.conversation_owner_account_id {
        return false;
    }
    match (&a.tenant, &a.funding) {
        (Tenant::Personal { account_id, .. }, Funding::Personal { account_id: payer })
            if account_id == payer => {}
        (
            Tenant::Team {
                team_id,
                account_id,
                ..
            },
            Funding::Team {
                team_id: payer_team,
                account_id: payer,
            },
        ) if team_id == payer_team && account_id == payer => {}
        (
            _,
            Funding::Sponsored {
                account_id,
                policy_id,
                policy_version,
            },
        ) if *account_id > 0 && label(policy_id) && *policy_version > 0 => {}
        _ => return false,
    }
    let mut names = BTreeSet::new();
    a.tools.iter().all(|t| {
        label(&t.name) && label(&t.version) && label(&t.price_version) && names.insert(&t.name)
    })
}

impl Grant {
    /// Build only after actual Core session/policy verification or a trusted
    /// persisted-row read. Structural validation cannot turn Go claims into
    /// authority. Random ID generation and durable insertion belong to Core.
    pub fn from_core_state(
        id: String,
        authority: Authority,
        issued_at: i64,
        expires_at: i64,
    ) -> Result<Self, Error> {
        if !grant_id(&id)
            || !valid_authority(&authority)
            || issued_at <= 0
            || expires_at <= issued_at
            || expires_at - issued_at > MAX_TTL_SECONDS
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            id,
            authority,
            issued_at,
            expires_at,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn authority(&self) -> &Authority {
        &self.authority
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}

/// Pure guard for a new attempt. `current` comes from an authenticated Session
/// plus live policy read under the same lock as revocation and budget mutation.
/// A revoked/expired grant cannot spend even when an earlier check succeeded.
/// Status-only recovery uses its separate owner-authenticated read path; it must
/// retain the original binding and never require a new spending grant.
pub fn check_attempt(
    grant: &Grant,
    state: CurrentState<'_>,
    invocation: &Invocation,
    computed_payload_sha256: [u8; 32],
    parsed_tools: &[ToolScope],
    existing: Option<&ExistingOperation>,
) -> Result<Decision, Error> {
    let CurrentState {
        authority: current,
        now,
        revoked,
        next_round,
        budget,
    } = state;
    if revoked || now < grant.issued_at || now >= grant.expires_at {
        return Err(Error::Unauthorized);
    }
    if !valid_authority(current) || current != &grant.authority {
        return Err(Error::Forbidden);
    }
    if invocation.version != VERSION
        || !label(&invocation.operation_key)
        || invocation.delegation_id != grant.id
        || invocation.conversation_id != current.conversation_id
        || invocation.turn_id != current.turn_id
        || invocation.round >= current.max_model_calls
        || invocation.operation_key != format!("{}:{}", current.turn_id, invocation.round)
    {
        return Err(Error::Forbidden);
    }
    if invocation.payload_sha256 != computed_payload_sha256 {
        return Err(Error::Conflict);
    }
    let mut names = BTreeSet::new();
    if parsed_tools
        .iter()
        .any(|tool| !names.insert(&tool.name) || !current.tools.contains(tool))
    {
        return Err(Error::Forbidden);
    }
    if let Some(old) = existing {
        if old.delegation_id != grant.id
            || old.operation_key != invocation.operation_key
            || old.payload_sha256 != invocation.payload_sha256
        {
            return Err(Error::Conflict);
        }
        return Ok(match old.status {
            Status::Settled | Status::Released => Decision::ReplayTerminal(old.status),
            state => Decision::InspectOriginal(state),
        });
    }
    if invocation.round != next_round {
        return Err(Error::Conflict);
    }
    if budget.held_credits < 0
        || budget.settled_credits < 0
        || budget.reserve_credits <= 0
        || budget
            .held_credits
            .checked_add(budget.settled_credits)
            .and_then(|used| used.checked_add(budget.reserve_credits))
            .is_none_or(|total| total > current.budget_credits)
    {
        return Err(Error::BudgetExceeded);
    }
    Ok(Decision::ReserveNew)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn authority() -> Authority {
        Authority {
            actor_user_id: 7,
            session_id: 11,
            auth_version: 3,
            tenant: Tenant::Personal {
                user_id: 7,
                account_id: 17,
            },
            conversation_id: "conversation-1".into(),
            conversation_owner_user_id: 7,
            conversation_owner_account_id: 17,
            turn_id: "turn-1".into(),
            funding: Funding::Personal { account_id: 17 },
            model: "fixture-model".into(),
            group: "default".into(),
            price_version: "price-1".into(),
            tools: vec![ToolScope {
                name: "tools.search".into(),
                version: "1".into(),
                price_version: "free-1".into(),
                schema_sha256: [2; 32],
            }],
            budget_credits: 100,
            max_model_calls: 4,
            revocation_version: 1,
        }
    }
    fn grant(a: Authority) -> Grant {
        Grant::from_core_state(format!("lmm_ad_{}", "a".repeat(64)), a, 1000, 1100).unwrap()
    }
    fn invoke(g: &Grant) -> Invocation {
        Invocation {
            version: VERSION,
            delegation_id: g.id.clone(),
            conversation_id: "conversation-1".into(),
            turn_id: "turn-1".into(),
            round: 0,
            operation_key: "turn-1:0".into(),
            payload_sha256: [3; 32],
        }
    }
    fn check(
        g: &Grant,
        a: &Authority,
        i: &Invocation,
        old: Option<&ExistingOperation>,
    ) -> Result<Decision, Error> {
        check_attempt(
            g,
            CurrentState {
                authority: a,
                now: 1050,
                revoked: false,
                next_round: 0,
                budget: Budget {
                    held_credits: 20,
                    settled_credits: 30,
                    reserve_credits: 50,
                },
            },
            i,
            [3; 32],
            &a.tools,
            old,
        )
    }
    #[test]
    fn valid_exact_budget_reserves() {
        let a = authority();
        let g = grant(a.clone());
        assert_eq!(check(&g, &a, &invoke(&g), None), Ok(Decision::ReserveNew));
    }
    #[test]
    fn each_live_binding_change_is_rejected() {
        let a = authority();
        let g = grant(a.clone());
        let i = invoke(&g);
        let mut cases = vec![];
        macro_rules! changed {
            ($field:ident, $value:expr) => {{
                let mut v = a.clone();
                v.$field = $value;
                cases.push(v);
            }};
        }
        changed!(actor_user_id, 8);
        changed!(session_id, 12);
        changed!(auth_version, 4);
        changed!(conversation_id, "conversation-2".into());
        changed!(conversation_owner_user_id, 8);
        changed!(conversation_owner_account_id, 18);
        changed!(turn_id, "turn-2".into());
        changed!(model, "other".into());
        changed!(group, "other".into());
        changed!(price_version, "price-2".into());
        changed!(
            funding,
            Funding::Sponsored {
                account_id: 99,
                policy_id: "sponsor".into(),
                policy_version: 1
            }
        );
        changed!(budget_credits, 101);
        changed!(max_model_calls, 3);
        changed!(revocation_version, 2);
        let mut t = a.clone();
        t.tools[0].schema_sha256 = [9; 32];
        cases.push(t);
        let mut t = a.clone();
        t.tools[0].version = "2".into();
        cases.push(t);
        let mut t = a.clone();
        t.tools[0].price_version = "price-2".into();
        cases.push(t);
        for current in cases {
            assert_eq!(check(&g, &current, &i, None), Err(Error::Forbidden));
        }
    }
    #[test]
    fn ownership_and_funding_cannot_cross_accounts_or_teams() {
        let mut a = authority();
        a.funding = Funding::Personal { account_id: 18 };
        assert!(!valid_authority(&a));
        a = authority();
        a.tenant = Tenant::Team {
            team_id: 5,
            account_id: 17,
            membership_version: 1,
        };
        assert!(!valid_authority(&a));
        a.funding = Funding::Team {
            team_id: 5,
            account_id: 17,
        };
        assert!(valid_authority(&a));
        a.funding = Funding::Team {
            team_id: 6,
            account_id: 17,
        };
        assert!(!valid_authority(&a));
        a = authority();
        a.conversation_owner_user_id = 8;
        assert!(!valid_authority(&a));
    }
    #[test]
    fn expiry_and_revocation_reject_at_dispatch_boundary() {
        let a = authority();
        let g = grant(a.clone());
        let i = invoke(&g);
        for (now, revoked) in [(999, false), (1100, false), (1050, true)] {
            assert_eq!(
                check_attempt(
                    &g,
                    CurrentState {
                        authority: &a,
                        now,
                        revoked,
                        next_round: 0,
                        budget: Budget {
                            held_credits: 0,
                            settled_credits: 0,
                            reserve_credits: 1,
                        }
                    },
                    &i,
                    [3; 32],
                    &a.tools,
                    None,
                ),
                Err(Error::Unauthorized)
            );
        }
    }
    #[test]
    fn conflicting_payload_and_operation_never_replay() {
        let a = authority();
        let g = grant(a.clone());
        let mut i = invoke(&g);
        i.payload_sha256 = [4; 32];
        assert_eq!(check(&g, &a, &i, None), Err(Error::Conflict));
        i = invoke(&g);
        for old in [
            ExistingOperation {
                delegation_id: "another".into(),
                operation_key: i.operation_key.clone(),
                payload_sha256: i.payload_sha256,
                status: Status::Settled,
            },
            ExistingOperation {
                delegation_id: g.id.clone(),
                operation_key: "turn-1:1".into(),
                payload_sha256: i.payload_sha256,
                status: Status::Unknown,
            },
            ExistingOperation {
                delegation_id: g.id.clone(),
                operation_key: i.operation_key.clone(),
                payload_sha256: [4; 32],
                status: Status::Settled,
            },
        ] {
            assert_eq!(check(&g, &a, &i, Some(&old)), Err(Error::Conflict));
        }
    }
    #[test]
    fn every_existing_nonterminal_state_requires_original_status() {
        let a = authority();
        let g = grant(a.clone());
        let i = invoke(&g);
        for state in [
            Status::Reserved,
            Status::Dispatched,
            Status::Streaming,
            Status::Unknown,
            Status::Settled,
            Status::Released,
        ] {
            let old = ExistingOperation {
                delegation_id: g.id.clone(),
                operation_key: i.operation_key.clone(),
                payload_sha256: i.payload_sha256,
                status: state,
            };
            let expected = if matches!(state, Status::Settled | Status::Released) {
                Decision::ReplayTerminal(state)
            } else {
                Decision::InspectOriginal(state)
            };
            assert_eq!(check(&g, &a, &i, Some(&old)), Ok(expected));
        }
    }
    #[test]
    fn cumulative_holds_and_overflow_never_reset_budget() {
        let a = authority();
        let g = grant(a.clone());
        let i = invoke(&g);
        for b in [
            Budget {
                held_credits: 21,
                settled_credits: 30,
                reserve_credits: 50,
            },
            Budget {
                held_credits: -1,
                settled_credits: 0,
                reserve_credits: 1,
            },
            Budget {
                held_credits: 0,
                settled_credits: 0,
                reserve_credits: 0,
            },
            Budget {
                held_credits: i64::MAX,
                settled_credits: 1,
                reserve_credits: 1,
            },
        ] {
            assert_eq!(
                check_attempt(
                    &g,
                    CurrentState {
                        authority: &a,
                        now: 1050,
                        revoked: false,
                        next_round: 0,
                        budget: b
                    },
                    &i,
                    [3; 32],
                    &a.tools,
                    None
                ),
                Err(Error::BudgetExceeded)
            );
        }
    }
    #[test]
    fn turn_round_and_parsed_tool_scope_are_checked() {
        let a = authority();
        let g = grant(a.clone());
        for (round, key) in [(4, "turn-1:4"), (0, "other-turn:0"), (0, "turn-1:00")] {
            let mut i = invoke(&g);
            i.round = round;
            i.operation_key = key.into();
            assert_eq!(check(&g, &a, &i, None), Err(Error::Forbidden));
        }
        let i = invoke(&g);
        let state = || CurrentState {
            authority: &a,
            now: 1050,
            revoked: false,
            next_round: 0,
            budget: Budget {
                held_credits: 0,
                settled_credits: 0,
                reserve_credits: 1,
            },
        };
        let mut tool = a.tools[0].clone();
        tool.schema_sha256 = [7; 32];
        for tools in [vec![tool], vec![a.tools[0].clone(), a.tools[0].clone()]] {
            assert_eq!(
                check_attempt(&g, state(), &i, [3; 32], &tools, None),
                Err(Error::Forbidden)
            );
        }
        assert_eq!(
            check_attempt(&g, state(), &i, [3; 32], &[], None),
            Ok(Decision::ReserveNew)
        );
        let mut next = invoke(&g);
        next.round = 1;
        next.operation_key = "turn-1:1".into();
        assert_eq!(
            check_attempt(&g, state(), &next, [3; 32], &[], None),
            Err(Error::Conflict)
        );
    }
    #[test]
    fn invalid_grants_and_duplicate_tools_are_not_authority() {
        for ttl in [0, MAX_TTL_SECONDS + 1] {
            assert!(
                Grant::from_core_state(
                    format!("lmm_ad_{}", "a".repeat(64)),
                    authority(),
                    1000,
                    1000 + ttl
                )
                .is_err()
            );
        }
        assert!(Grant::from_core_state("root-key".into(), authority(), 1000, 1100).is_err());
        let mut a = authority();
        a.tools.push(a.tools[0].clone());
        assert!(!valid_authority(&a));
        a = authority();
        a.budget_credits = MAX_CREDITS + 1;
        assert!(!valid_authority(&a));
    }
}
