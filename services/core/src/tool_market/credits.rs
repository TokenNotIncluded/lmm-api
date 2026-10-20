//! Legacy CREDIT v2 price snapshots. These units are existing wallet credits,
//! never native MCP USD millionths. Source: frozen main 3948a0a model/
//! tool_market_{billing,metering,usage}.go (AGPL-3.0-or-later).
//! This leaf computes economics only; all wallet mutation remains in Core ledger.
use super::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_CREDITS: i64 = 9_007_199_254_740_991;
pub const SCHEMA_V2: &str = concat!(
    include_str!("credits_schema.sql"),
    "\n",
    include_str!("credit_meta_schema.sql")
);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub metric: String,
    pub rate_quota: i64,
    pub max_quantity: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Price {
    Fixed { price_quota: i64 },
    Metered { rules: Vec<Rule> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    ToolReported,
    PlatformVerified,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub source: UsageSource,
    pub call_id: String,
    pub version_id: String,
    pub tool_id: String,
    pub input_digest: String,
    pub pricing_digest: String,
    pub result_digest: String,
    pub quantities: BTreeMap<String, i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quote {
    pub id: String,
    pub service_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub remote_digest: String,
    pub pricing_digest: String,
    pub price: Price,
    pub expires: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consent {
    pub id: String,
    pub client_id: String,
    pub quote_id: String,
    pub max_price_quota: i64,
    pub max_total_quota: i64,
    pub max_calls: i64,
    pub expires: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reserve {
    pub client_id: String,
    pub request_key: String,
    pub quote_id: String,
    pub grant_id: String,
    pub tool_id: String,
    pub version_id: String,
    pub input_digest: String,
    pub resolve_by: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Economics {
    pub held: i64,
    pub charged: i64,
    pub refunded: i64,
    pub seller: i64,
    pub fee: i64,
}
fn bounded(v: i64) -> bool {
    (0..=MAX_CREDITS).contains(&v)
}
fn scale(metric: &str) -> Option<i64> {
    Some(match metric {
        "input_tokens" | "output_tokens" => 1_000_000,
        "input_characters"
        | "output_characters"
        | "audio_milliseconds"
        | "video_milliseconds"
        | "cpu_core_milliseconds"
        | "gpu_milliseconds"
        | "vm_milliseconds" => 1_000,
        "images" => 1,
        "memory_mib_seconds" => 1_024,
        "storage_mib_seconds" => 3_686_400,
        _ => return None,
    })
}
/// Round each metric separately, as frozen main does. Combining numerators
/// before rounding undercharges independent metrics and is not equivalent.
pub fn rules_quota(rules: &[Rule], actual: Option<&BTreeMap<String, i64>>) -> Result<i64> {
    if rules.is_empty() || rules.len() > 12 || actual.is_some_and(|q| q.len() != rules.len()) {
        return Err(Error::Invalid);
    }
    let mut seen = BTreeSet::new();
    let mut total = 0_i128;
    for rule in rules {
        let scale = scale(&rule.metric).ok_or(Error::Invalid)?;
        if !seen.insert(&rule.metric)
            || rule.rate_quota <= 0
            || !bounded(rule.rate_quota)
            || rule.max_quantity <= 0
            || rule.max_quantity > 1_000_000_000_000
        {
            return Err(Error::Invalid);
        }
        let quantity = match actual {
            Some(q) => *q.get(&rule.metric).ok_or(Error::Invalid)?,
            None => rule.max_quantity,
        };
        if quantity < 0 || quantity > rule.max_quantity {
            return Err(Error::Invalid);
        }
        let numerator = i128::from(rule.rate_quota) * i128::from(quantity);
        total += (numerator + i128::from(scale) - 1) / i128::from(scale);
        if total > i128::from(MAX_CREDITS) {
            return Err(Error::Invalid);
        }
    }
    i64::try_from(total).map_err(|_| Error::Invalid)
}
impl Price {
    pub fn ceiling(&self) -> Result<i64> {
        match self {
            Self::Fixed { price_quota } if bounded(*price_quota) => Ok(*price_quota),
            Self::Metered { rules } => rules_quota(rules, None),
            _ => Err(Error::Invalid),
        }
    }
    pub fn charge(&self, quantities: Option<&BTreeMap<String, i64>>) -> Result<i64> {
        match self {
            Self::Fixed { .. } if quantities.is_none() => self.ceiling(),
            Self::Metered { rules } => rules_quota(rules, Some(quantities.ok_or(Error::Invalid)?)),
            _ => Err(Error::Invalid),
        }
    }
}
/// Immutable installer policy supplies fee_bps/platform account. Neither the
/// publisher nor the Go execution host may select a new payout policy at settle.
pub fn economics(
    price: &Price,
    quantities: Option<&BTreeMap<String, i64>>,
    fee_bps: i64,
) -> Result<Economics> {
    if !(0..=10_000).contains(&fee_bps) {
        return Err(Error::Invalid);
    }
    let held = price.ceiling()?;
    let charged = price.charge(quantities)?;
    if charged > held {
        return Err(Error::Invalid);
    }
    let fee = i64::try_from(i128::from(charged) * i128::from(fee_bps) / 10_000)
        .map_err(|_| Error::Invalid)?;
    Ok(Economics {
        held,
        charged,
        refunded: held - charged,
        seller: charged - fee,
        fee,
    })
}
impl Consent {
    pub fn validate(&self, quote: &Quote, now: i64) -> Result<()> {
        if !super::name(&self.id)
            || !super::name(&self.client_id)
            || self.quote_id != quote.id
            || self.expires <= now
            || self.expires > quote.expires
            || self.max_calls <= 0
            || self.max_calls > 1_000_000
            || !bounded(self.max_price_quota)
            || !bounded(self.max_total_quota)
            || quote.price.ceiling()? > self.max_price_quota
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_metric_rounds_independently_and_fee_floors() {
        let rules = vec![
            Rule {
                metric: "input_tokens".into(),
                rate_quota: 1,
                max_quantity: 1,
            },
            Rule {
                metric: "output_tokens".into(),
                rate_quota: 1,
                max_quantity: 1,
            },
        ];
        assert_eq!(rules_quota(&rules, None), Ok(2));
        let actual = BTreeMap::from([("input_tokens".into(), 1), ("output_tokens".into(), 0)]);
        assert_eq!(
            economics(&Price::Metered { rules }, Some(&actual), 2500),
            Ok(Economics {
                held: 2,
                charged: 1,
                refunded: 1,
                seller: 1,
                fee: 0
            })
        );
        let fixed = Price::Fixed { price_quota: 101 };
        assert_eq!(
            economics(&fixed, None, 2500),
            Ok(Economics {
                held: 101,
                charged: 101,
                refunded: 0,
                seller: 76,
                fee: 25
            })
        );
    }
    #[test]
    fn unknown_extra_duplicate_negative_overflow_usage_is_rejected() {
        let r = Rule {
            metric: "images".into(),
            rate_quota: MAX_CREDITS,
            max_quantity: 2,
        };
        assert_eq!(rules_quota(&[r.clone()], None), Err(Error::Invalid));
        assert_eq!(
            rules_quota(&[r.clone(), r.clone()], None),
            Err(Error::Invalid)
        );
        let mut r = r;
        r.max_quantity = 1;
        assert_eq!(
            rules_quota(&[r.clone()], Some(&BTreeMap::from([("images".into(), -1)]))),
            Err(Error::Invalid)
        );
        assert_eq!(
            rules_quota(
                &[r.clone()],
                Some(&BTreeMap::from([("images".into(), 1), ("extra".into(), 1)]))
            ),
            Err(Error::Invalid)
        );
        r.metric = "unknown".into();
        assert_eq!(rules_quota(&[r], None), Err(Error::Invalid));
        assert_eq!(
            economics(&Price::Fixed { price_quota: 1 }, None, 10001),
            Err(Error::Invalid)
        );
    }
}
