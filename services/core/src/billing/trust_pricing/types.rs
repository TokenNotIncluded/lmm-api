use super::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
const MAX_AT: i64 = 253_402_300_799;
const MAX_DENOMINATOR: u64 = 1_000_000_000_000_000_000;

/// Exact bounded ratio. Construction rejects zero, increases and rounding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawRatio", into = "RawRatio")]
pub struct DiscountRatio {
    numerator: u64,
    denominator: u64,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRatio {
    numerator: u64,
    denominator: u64,
}
impl TryFrom<RawRatio> for DiscountRatio {
    type Error = &'static str;
    fn try_from(r: RawRatio) -> std::result::Result<Self, Self::Error> {
        Self::new(r.numerator, r.denominator).map_err(|_| "invalid trust ratio")
    }
}
impl From<DiscountRatio> for RawRatio {
    fn from(r: DiscountRatio) -> Self {
        Self {
            numerator: r.numerator,
            denominator: r.denominator,
        }
    }
}
impl DiscountRatio {
    pub fn new(numerator: u64, denominator: u64) -> Result<Self> {
        if numerator == 0
            || denominator == 0
            || numerator > denominator
            || denominator > MAX_DENOMINATOR
        {
            return Err(Error::Invalid);
        }
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Self {
            numerator: numerator / a,
            denominator: denominator / a,
        })
    }
    pub fn numerator(self) -> u64 {
        self.numerator
    }
    pub fn denominator(self) -> u64 {
        self.denominator
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustTier {
    pub level: u8,
    pub min_paid_credits: i64,
    pub discount: DiscountRatio,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustPolicy {
    pub version: u32,
    pub paid_activation_enabled: bool,
    pub decay_period_days: u16,
    pub tiers: [TrustTier; 5],
    pub admin_discount: DiscountRatio,
    pub superadmin_discount: DiscountRatio,
}
impl TrustPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.decay_period_days > 3650 {
            return Err(Error::Invalid);
        }
        for (level, tier) in self.tiers.iter().enumerate() {
            if usize::from(tier.level) != level
                || tier.min_paid_credits < 0
                || tier.min_paid_credits > i64::from(i32::MAX)
                || (level == 0 && tier.min_paid_credits != 0)
                || (level > 0 && tier.min_paid_credits <= self.tiers[level - 1].min_paid_credits)
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
/// Stored with a trusted charging price, never accepted as authority for a new
/// quote. Only UserPriceAuthority is usable for new admission. Deserialize here
/// supports restart of ALREADY persisted server-owned request evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenTrustPrice {
    user_id: i64,
    policy_revision: i64,
    basis_revision: i64,
    effective_level: u8,
    level_source: String,
    discount: DiscountRatio,
    evaluated_at: i64,
    next_decay_at: Option<i64>,
    evidence_digest: [u8; 32],
}
impl FrozenTrustPrice {
    pub fn validate(&self) -> Result<()> {
        if self.user_id <= 0
            || self.policy_revision <= 0
            || self.basis_revision < 0
            || self.effective_level > 6
            || !valid_at(self.evaluated_at)
            || self
                .next_decay_at
                .is_some_and(|v| !valid_at(v) || v <= self.evaluated_at)
            || self.evidence_digest == [0; 32]
            || !matches!(
                self.level_source.as_str(),
                "role" | "override" | "automatic"
            )
            || (self.effective_level >= 5) != (self.level_source == "role")
            || (self.level_source != "automatic" && self.next_decay_at.is_some())
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn discount(&self) -> DiscountRatio {
        self.discount
    }
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
    pub fn level(&self) -> u8 {
        self.effective_level
    }
    pub fn next_decay_at(&self) -> Option<i64> {
        self.next_decay_at
    }
}
/// No Deserialize and no public constructor: only the same-transaction Core
/// resolver can mint a new-user quote. Go/HTTP fields cannot become this type.
#[derive(Clone, Debug)]
pub(crate) struct UserPriceAuthority {
    frozen: FrozenTrustPrice,
}
impl UserPriceAuthority {
    pub(crate) fn discount(&self) -> DiscountRatio {
        self.frozen.discount
    }
    pub(crate) fn evidence(&self) -> &FrozenTrustPrice {
        &self.frozen
    }
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct Facts {
    pub user_id: i64,
    pub role: String,
    pub basis_revision: i64,
    pub created_at: i64,
    pub activated_at: i64,
    pub last_api_at: i64,
    pub override_level: Option<u8>,
    pub projection_known: bool,
    pub net_paid_credits: i64,
    pub positive_orders: i64,
    pub last_paid_at: i64,
    pub evidence: serde_json::Value,
}
pub(super) fn valid_at(at: i64) -> bool {
    (0..=MAX_AT).contains(&at)
}
pub(super) fn evaluate(
    policy: &TrustPolicy,
    revision: i64,
    f: &Facts,
    at: i64,
) -> Result<UserPriceAuthority> {
    policy.validate()?;
    if f.user_id <= 0
        || revision <= 0
        || f.basis_revision < 0
        || !valid_at(at)
        || [f.created_at, f.activated_at, f.last_api_at, f.last_paid_at]
            .into_iter()
            .any(|v| !valid_at(v) || v > at)
        || f.net_paid_credits < 0
        || f.positive_orders < 0
        || (f.positive_orders == 0 && (f.net_paid_credits != 0 || f.last_paid_at != 0))
        || (f.positive_orders > 0 && (f.net_paid_credits == 0 || f.last_paid_at == 0))
        || f.override_level.is_some_and(|v| v > 4)
    {
        return Err(Error::Invalid);
    }
    let (level, source, next) = match f.role.as_str() {
        "superadmin" => (6, "role", None),
        "admin" => (5, "role", None),
        "user" => {
            if let Some(level) = f.override_level {
                (level, "override", None)
            } else {
                if !f.projection_known {
                    return Err(Error::Unavailable);
                }
                let activated = f.activated_at > 0
                    || (policy.paid_activation_enabled
                        && f.positive_orders > 0
                        && f.net_paid_credits >= policy.tiers[1].min_paid_credits);
                let automatic = if !activated {
                    0
                } else {
                    (2..=4)
                        .rev()
                        .find(|level| f.net_paid_credits >= policy.tiers[*level].min_paid_credits)
                        .unwrap_or(1)
                };
                let anchor = f.created_at.max(f.last_api_at).max(f.last_paid_at);
                let period = i64::from(policy.decay_period_days) * 86400;
                let steps = if automatic > 1 && anchor > 0 && period > 0 {
                    ((at - anchor) / period).min((automatic - 1) as i64)
                } else {
                    0
                };
                let level = automatic as u8 - steps as u8;
                let next = if level > 1 && anchor > 0 && period > 0 {
                    Some(
                        anchor
                            .checked_add((steps + 1).checked_mul(period).ok_or(Error::Invalid)?)
                            .filter(|v| valid_at(*v))
                            .ok_or(Error::Invalid)?,
                    )
                } else {
                    None
                };
                (level, "automatic", next)
            }
        }
        _ => return Err(Error::Invalid),
    };
    let discount = match level {
        6 => policy.superadmin_discount,
        5 => policy.admin_discount,
        v => policy.tiers[usize::from(v)].discount,
    };
    let digest =
        Sha256::digest(serde_json::to_vec(&(policy, revision, f, at)).map_err(|_| Error::Invalid)?)
            .into();
    let frozen = FrozenTrustPrice {
        user_id: f.user_id,
        policy_revision: revision,
        basis_revision: f.basis_revision,
        effective_level: level,
        level_source: source.into(),
        discount,
        evaluated_at: at,
        next_decay_at: next,
        evidence_digest: digest,
    };
    frozen.validate()?;
    Ok(UserPriceAuthority { frozen })
}
