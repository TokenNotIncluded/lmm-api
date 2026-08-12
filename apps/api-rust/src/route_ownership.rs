//! Closed-by-default Rust ownership eligibility evidence.
//!
//! This module records a route-specific readiness decision without mounting a
//! handler or transferring production traffic.  A caller must prove every
//! required Go-vs-Rust differential class, an identical shadow result, a
//! minimum canary allocation, and an explicit rollout approval before the
//! result can become eligible for an independent ownership review.

use std::{collections::BTreeSet, error::Error, fmt};

use lmm_contracts::relay::Protocol;
use serde::{Deserialize, Serialize};

/// Maximum canary allocation represented in basis points.
pub const MAX_CANARY_BASIS_POINTS: u16 = 10_000;

/// Minimum canary allocation required before an ownership review can open.
pub const MIN_REVIEW_CANARY_BASIS_POINTS: u16 = 100;

/// Differential classes which must all be green for one route.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DifferentialClass {
    /// Non-streaming request/response behavior.
    NonStream,
    /// Streaming framing, ordering, and termination behavior.
    Stream,
    /// Error status and error-body behavior.
    Error,
    /// Usage, cache, and billing semantics.
    UsageBilling,
    /// Client disconnect and cancellation behavior.
    ClientAbort,
}

impl DifferentialClass {
    /// Stable declaration-order list of required differential classes.
    pub const ALL: [Self; 5] = [
        Self::NonStream,
        Self::Stream,
        Self::Error,
        Self::UsageBilling,
        Self::ClientAbort,
    ];

    /// Returns every class in deterministic gate order.
    pub const fn all() -> &'static [Self] {
        &Self::ALL
    }
}

/// A route identity attached to ownership evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteOwnershipScope {
    /// Protocol accepted by the candidate route.
    pub source: Protocol,
    /// Protocol emitted by the candidate route.
    pub target: Protocol,
    /// Whether the route's contract includes a streaming response.
    pub stream: bool,
}

/// Safe reasons why a route remains closed to ownership review.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipBlocker {
    /// One or more differential classes have not passed.
    DifferentialNotGreen,
    /// Shadow conversion did not produce identical body-free summaries.
    ShadowDifference,
    /// Canary allocation is below the review threshold.
    CanaryBelowMinimum,
    /// A human/operator rollout approval has not been recorded.
    RolloutNotApproved,
    /// Evidence contains a canary value outside the closed range.
    InvalidCanary,
}

/// Evidence collected for one specific source/target route.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnershipEvidence {
    scope: RouteOwnershipScope,
    green: BTreeSet<DifferentialClass>,
    shadow_identical: bool,
    canary_basis_points: u16,
    rollout_approved: bool,
}

impl OwnershipEvidence {
    /// Creates closed evidence for one route. No class or approval is green.
    #[must_use]
    pub fn closed(scope: RouteOwnershipScope) -> Self {
        Self {
            scope,
            green: BTreeSet::new(),
            shadow_identical: false,
            canary_basis_points: 0,
            rollout_approved: false,
        }
    }

    /// Returns the route identity attached to this evidence.
    #[must_use]
    pub const fn scope(&self) -> RouteOwnershipScope {
        self.scope
    }

    /// Marks one differential class green after its external comparison passes.
    pub fn mark_green(&mut self, class: DifferentialClass) {
        self.green.insert(class);
    }

    /// Returns whether a differential class has passed.
    #[must_use]
    pub fn is_green(&self, class: DifferentialClass) -> bool {
        self.green.contains(&class)
    }

    /// Records whether body-free old/new shadow summaries were identical.
    pub fn set_shadow_identical(&mut self, identical: bool) {
        self.shadow_identical = identical;
    }

    /// Records a bounded canary allocation.
    pub fn set_canary_basis_points(
        &mut self,
        basis_points: u16,
    ) -> Result<(), OwnershipEvidenceError> {
        if basis_points > MAX_CANARY_BASIS_POINTS {
            return Err(OwnershipEvidenceError::InvalidCanary { basis_points });
        }
        self.canary_basis_points = basis_points;
        Ok(())
    }

    /// Records the explicit operator approval required by the gate.
    pub fn approve_rollout(&mut self) {
        self.rollout_approved = true;
    }

    /// Returns the classes currently recorded as green.
    #[must_use]
    pub fn green_classes(&self) -> &BTreeSet<DifferentialClass> {
        &self.green
    }

    /// Returns the canary allocation in basis points.
    #[must_use]
    pub const fn canary_basis_points(&self) -> u16 {
        self.canary_basis_points
    }

    /// Returns whether explicit rollout approval was recorded.
    #[must_use]
    pub const fn rollout_approved(&self) -> bool {
        self.rollout_approved
    }
}

/// Configures the minimum proof required for one ownership review.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnershipGate {
    minimum_canary_basis_points: u16,
}

impl Default for OwnershipGate {
    fn default() -> Self {
        Self {
            minimum_canary_basis_points: MIN_REVIEW_CANARY_BASIS_POINTS,
        }
    }
}

impl OwnershipGate {
    /// Creates a gate with a bounded canary threshold.
    pub fn new(minimum_canary_basis_points: u16) -> Result<Self, OwnershipEvidenceError> {
        if minimum_canary_basis_points > MAX_CANARY_BASIS_POINTS {
            return Err(OwnershipEvidenceError::InvalidCanary {
                basis_points: minimum_canary_basis_points,
            });
        }
        Ok(Self {
            minimum_canary_basis_points,
        })
    }

    /// Evaluates evidence without mounting or taking ownership of any route.
    #[must_use]
    pub fn evaluate(&self, evidence: &OwnershipEvidence) -> OwnershipDecision {
        let mut blockers = Vec::new();
        if evidence.canary_basis_points > MAX_CANARY_BASIS_POINTS {
            blockers.push(OwnershipBlocker::InvalidCanary);
        }
        if DifferentialClass::all()
            .iter()
            .any(|class| !evidence.green.contains(class))
        {
            blockers.push(OwnershipBlocker::DifferentialNotGreen);
        }
        if !evidence.shadow_identical {
            blockers.push(OwnershipBlocker::ShadowDifference);
        }
        if evidence.canary_basis_points < self.minimum_canary_basis_points {
            blockers.push(OwnershipBlocker::CanaryBelowMinimum);
        }
        if !evidence.rollout_approved {
            blockers.push(OwnershipBlocker::RolloutNotApproved);
        }
        if blockers.is_empty() {
            OwnershipDecision::EligibleForOwnershipReview {
                scope: evidence.scope,
            }
        } else {
            OwnershipDecision::ClosedByDefault {
                scope: evidence.scope,
                blockers,
            }
        }
    }
}

/// Result of the closed ownership gate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OwnershipDecision {
    /// Evidence is incomplete; production ownership remains closed.
    ClosedByDefault {
        /// Route whose evidence was evaluated.
        scope: RouteOwnershipScope,
        /// Every blocker found by the pure evaluation.
        blockers: Vec<OwnershipBlocker>,
    },
    /// Evidence is sufficient to open a separate ownership review.
    ///
    /// This is not a route takeover command and has no side effect.
    EligibleForOwnershipReview {
        /// Route which may be reviewed by an independent deployment process.
        scope: RouteOwnershipScope,
    },
}

/// Errors from constructing bounded ownership evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OwnershipEvidenceError {
    /// Canary basis points exceeded the closed range.
    InvalidCanary {
        /// Rejected value.
        basis_points: u16,
    },
}

impl fmt::Display for OwnershipEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCanary { basis_points } => write!(
                formatter,
                "ownership canary basis points {basis_points} exceed {MAX_CANARY_BASIS_POINTS}"
            ),
        }
    }
}

impl Error for OwnershipEvidenceError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> RouteOwnershipScope {
        RouteOwnershipScope {
            source: Protocol::OpenAi,
            target: Protocol::OpenAi,
            stream: true,
        }
    }

    fn complete() -> OwnershipEvidence {
        let mut evidence = OwnershipEvidence::closed(scope());
        for class in DifferentialClass::all() {
            evidence.mark_green(*class);
        }
        evidence.set_shadow_identical(true);
        evidence
            .set_canary_basis_points(MIN_REVIEW_CANARY_BASIS_POINTS)
            .expect("bounded canary");
        evidence.approve_rollout();
        evidence
    }

    #[test]
    fn default_is_closed_and_complete_evidence_only_opens_review() {
        let gate = OwnershipGate::default();
        assert!(matches!(
            gate.evaluate(&OwnershipEvidence::closed(scope())),
            OwnershipDecision::ClosedByDefault { .. }
        ));
        assert_eq!(
            gate.evaluate(&complete()),
            OwnershipDecision::EligibleForOwnershipReview { scope: scope() }
        );
    }

    #[test]
    fn invalid_canary_is_rejected_without_opening_review() {
        assert!(OwnershipGate::new(MAX_CANARY_BASIS_POINTS + 1).is_err());
        let mut evidence = complete();
        assert!(
            evidence
                .set_canary_basis_points(MAX_CANARY_BASIS_POINTS + 1)
                .is_err()
        );
        assert_eq!(
            evidence.canary_basis_points(),
            MIN_REVIEW_CANARY_BASIS_POINTS
        );
    }
}
