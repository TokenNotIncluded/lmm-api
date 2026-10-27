//! Durable, internal-only admission evidence. This is not an accepted usage receipt.
use super::{Charge, Error, ReservationBasis, Usage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const EVENT_ID: &str = "reservation-contract:v1";
pub(crate) const ADMISSION_EVENT_ID: &str = "reservation-admission:v1";

/// A separate immutable admission record makes missing proof distinguishable
/// from an old request that never had a prepared contract. Both records are
/// inserted in the reserve transaction; neither is provider usage authority.
#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionMarker {
    kind: String,
    version: u32,
    prepared_sha256: String,
    admission_spec_sha256: String,
    frozen_price_sha256: String,
}

// @context-fold begin reservation-contract-types
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReservationBinding {
    pub route_id: String,
    pub target_id: u64,
    pub upstream_id: u64,
    pub protocol: String,
    pub model_snapshot: String,
    pub generation_url: String,
    pub credential_ref: crate::routing::CredentialRef,
    pub payload_policy_sha256: String,
    pub provider_contract_revision: String,
    pub projection_revision: String,
    pub quote_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReservationBounds {
    pub model_input_max: u64,
    pub input: u64,
    pub requested_output: u64,
    pub output: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReservationSemantics {
    pub input: String,
    pub output: String,
    pub missing_cached_tokens_is_zero: bool,
    pub missing_cache_creation_tokens_is_zero: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CounterEvidence {
    pub api: String,
    pub input_tokens: u64,
    pub request_sha256: String,
    pub response_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedFields {
    kind: String,
    version: u32,
    mode: String,
    request_fingerprint: String,
    binding: ReservationBinding,
    wire_sha256: String,
    counter: Option<CounterEvidence>,
    bounds: ReservationBounds,
    semantics: ReservationSemantics,
}

/// Only the gateway's preparation step constructs this type; public charging APIs do not accept it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct PreparedReservation {
    fields: PreparedFields,
    prepared_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReservationContract {
    fields: PreparedFields,
    prepared_sha256: String,
    admission_spec_sha256: String,
    frozen_price_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractWire {
    kind: String,
    version: u32,
    mode: String,
    request_fingerprint: String,
    binding: ReservationBinding,
    wire_sha256: String,
    counter: Option<CounterEvidence>,
    bounds: ReservationBounds,
    semantics: ReservationSemantics,
    prepared_sha256: String,
    admission_spec_sha256: String,
    frozen_price_sha256: String,
}

impl ReservationContract {
    // @context-fold begin contract-flat-json
    fn to_value(&self) -> Result<serde_json::Value, Error> {
        let mut value = serde_json::to_value(&self.fields).map_err(|_| Error::Invalid)?;
        value["prepared_sha256"] = self.prepared_sha256.clone().into();
        value["admission_spec_sha256"] = self.admission_spec_sha256.clone().into();
        value["frozen_price_sha256"] = self.frozen_price_sha256.clone().into();
        Ok(value)
    }

    fn from_value(value: serde_json::Value) -> Result<Self, Error> {
        if !value
            .as_object()
            .is_some_and(|object| object.contains_key("counter"))
        {
            return Err(Error::Invalid);
        }
        let wire: ContractWire = serde_json::from_value(value).map_err(|_| Error::Invalid)?;
        Ok(Self {
            fields: PreparedFields {
                kind: wire.kind,
                version: wire.version,
                mode: wire.mode,
                request_fingerprint: wire.request_fingerprint,
                binding: wire.binding,
                wire_sha256: wire.wire_sha256,
                counter: wire.counter,
                bounds: wire.bounds,
                semantics: wire.semantics,
            },
            prepared_sha256: wire.prepared_sha256,
            admission_spec_sha256: wire.admission_spec_sha256,
            frozen_price_sha256: wire.frozen_price_sha256,
        })
    }

    fn admission_marker(&self) -> AdmissionMarker {
        AdmissionMarker {
            kind: "model_reservation_admission".into(),
            version: 1,
            prepared_sha256: self.prepared_sha256.clone(),
            admission_spec_sha256: self.admission_spec_sha256.clone(),
            frozen_price_sha256: self.frozen_price_sha256.clone(),
        }
    }
    // @context-fold end contract-flat-json
}

pub(crate) fn hex_digest(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub(crate) fn typed_digest(value: &impl Serialize) -> Result<String, Error> {
    Ok(digest(
        &serde_json::to_vec(value).map_err(|_| Error::Invalid)?,
    ))
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
// @context-fold end reservation-contract-types

impl PreparedReservation {
    // @context-fold begin prepared-reservation-construction
    pub(crate) fn context_max(
        request_fingerprint: String,
        binding: ReservationBinding,
        wire_sha256: String,
        bounds: ReservationBounds,
        semantics: ReservationSemantics,
    ) -> Result<Self, Error> {
        Self::new(
            "context_max",
            request_fingerprint,
            binding,
            wire_sha256,
            None,
            bounds,
            semantics,
        )
    }

    pub(crate) fn exact(
        request_fingerprint: String,
        binding: ReservationBinding,
        wire_sha256: String,
        counter: CounterEvidence,
        bounds: ReservationBounds,
        semantics: ReservationSemantics,
    ) -> Result<Self, Error> {
        Self::new(
            "openai_responses_exact_v1",
            request_fingerprint,
            binding,
            wire_sha256,
            Some(counter),
            bounds,
            semantics,
        )
    }

    /// The existing native System One contract prices input only. Its reported
    /// output count is unpriced metadata, not a requested generative token cap.
    pub(crate) fn systemone_input_only(
        request_fingerprint: String,
        binding: ReservationBinding,
        wire_sha256: String,
        input_max: u64,
        mut semantics: ReservationSemantics,
    ) -> Result<Self, Error> {
        semantics.output = "unpriced_systemone_metadata".into();
        Self::new(
            "systemone_native_v1",
            request_fingerprint,
            binding,
            wire_sha256,
            None,
            ReservationBounds {
                model_input_max: input_max,
                input: input_max,
                requested_output: 0,
                output: u64::from(u32::MAX),
            },
            semantics,
        )
    }

    fn new(
        mode: &str,
        request_fingerprint: String,
        binding: ReservationBinding,
        wire_sha256: String,
        counter: Option<CounterEvidence>,
        bounds: ReservationBounds,
        semantics: ReservationSemantics,
    ) -> Result<Self, Error> {
        let fields = PreparedFields {
            kind: "model_reservation_contract".into(),
            version: 1,
            mode: mode.into(),
            request_fingerprint,
            binding,
            wire_sha256,
            counter,
            bounds,
            semantics,
        };
        let proof = Self {
            prepared_sha256: typed_digest(&fields)?,
            fields,
        };
        proof.validate()?;
        Ok(proof)
    }

    fn validate(&self) -> Result<(), Error> {
        let f = &self.fields;
        let binding = &f.binding;
        let bounds = &f.bounds;
        let native = f.mode == "systemone_native_v1";
        let valid_mode = match (f.mode.as_str(), &f.counter) {
            ("context_max", None) => {
                bounds.input == bounds.model_input_max && binding.protocol != "systemone"
            }
            ("openai_responses_exact_v1", Some(counter)) => {
                binding.protocol == "responses"
                    && counter.api == "responses.input_tokens"
                    && counter.input_tokens == bounds.input
                    && is_digest(&counter.request_sha256)
                    && is_digest(&counter.response_sha256)
            }
            ("systemone_native_v1", None) => {
                binding.protocol == "systemone"
                    && bounds.input == bounds.model_input_max
                    && bounds.requested_output == 0
                    && bounds.output == u64::from(u32::MAX)
            }
            _ => false,
        };
        if f.kind != "model_reservation_contract"
            || f.version != 1
            || !valid_mode
            || !is_digest(&f.request_fingerprint)
            || !is_digest(&f.wire_sha256)
            || !is_digest(&binding.payload_policy_sha256)
            || binding.route_id.is_empty()
            || binding.target_id == 0
            || binding.target_id > i64::MAX as u64
            || binding.upstream_id == 0
            || binding.upstream_id > i64::MAX as u64
            || binding.credential_ref.id == 0
            || binding.credential_ref.revision == 0
            || binding.quote_version == 0
            || !matches!(
                binding.protocol.as_str(),
                "chat" | "responses" | "anthropic" | "gemini" | "systemone"
            )
            || binding.model_snapshot.is_empty()
            || binding.generation_url.is_empty()
            || binding.provider_contract_revision.is_empty()
            || binding.projection_revision.is_empty()
            || bounds.model_input_max == 0
            || bounds.model_input_max > u32::MAX as u64
            || bounds.input > bounds.model_input_max
            || bounds.output == 0
            || bounds.output > u32::MAX as u64
            || (!native && bounds.output != bounds.requested_output)
            || !matches!(f.semantics.input.as_str(), "inclusive" | "exclusive")
            || f.semantics.output
                != if native {
                    "unpriced_systemone_metadata"
                } else {
                    "includes_reasoning"
                }
            || self.prepared_sha256 != typed_digest(f)?
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    pub(crate) fn verify_basis(&self, basis: ReservationBasis) -> Result<(), Error> {
        self.validate()?;
        let priced_output = if self.fields.mode == "systemone_native_v1" {
            0
        } else {
            self.fields.bounds.output
        };
        match basis {
            ReservationBasis::Tokens { input, output }
                if input == self.fields.bounds.input && output == priced_output =>
            {
                Ok(())
            }
            ReservationBasis::UpstreamCost { .. } => Ok(()),
            _ => Err(Error::Conflict),
        }
    }

    pub(crate) fn digest(&self) -> &str {
        &self.prepared_sha256
    }

    pub(crate) fn verify_request(
        &self,
        request: &super::Request,
        basis: ReservationBasis,
    ) -> Result<(), Error> {
        self.verify_basis(basis)?;
        self.validate_native_price(&request.price)?;
        if self.fields.request_fingerprint
            != request
                .fingerprint
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
            || request.reserve != request.price.reservation_ceiling(basis)?
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    pub(crate) fn freeze(&self, charge: &Charge, spec: &str) -> Result<ReservationContract, Error> {
        self.validate()?;
        self.validate_native_price(&charge.price)?;
        if !is_digest(spec) {
            return Err(Error::Conflict);
        }
        Ok(ReservationContract {
            fields: self.fields.clone(),
            prepared_sha256: self.prepared_sha256.clone(),
            admission_spec_sha256: spec.into(),
            frozen_price_sha256: typed_digest(&charge.price)?,
        })
    }

    fn validate_native_price(&self, price: &super::Price) -> Result<(), Error> {
        if self.fields.mode != "systemone_native_v1" {
            return Ok(());
        }
        price.validate()?;
        let scalar = price.tiered.as_ref().is_none_or(|stored| {
            let tariff = &stored.tariff;
            tariff.revision == 1
                && !tariff.uses_cache_read
                && !tariff.uses_cache_write
                && !tariff.uses_cache_write_1h
                && !tariff.uses_cache_write_12h
                && tariff.tiers.len() == 1
                && tariff.tiers[0].up_to_input_tokens.is_none()
                && tariff.tiers[0].rates.output.numerator == 0
                && tariff.tiers[0].rates.cache_read.numerator == 0
                && tariff.tiers[0].rates.cache_write.numerator == 0
        });
        if price.image.is_some()
            || price.upstream_cost_group_ppm.is_some()
            || price.output_per_million != 0
            || price.cached_per_million != 0
            || !scalar
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    // @context-fold end prepared-reservation-construction
}

impl ReservationContract {
    // @context-fold begin frozen-contract-validation
    pub(crate) fn validate(&self, charge: &Charge) -> Result<(), Error> {
        let proof = PreparedReservation {
            fields: self.fields.clone(),
            prepared_sha256: self.prepared_sha256.clone(),
        };
        proof.validate()?;
        if proof.freeze(charge, &self.admission_spec_sha256)? != *self {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    pub(crate) fn check_usage(&self, charge: &Charge, usage: Usage) -> Result<(), Error> {
        self.validate(charge)?;
        let bounds = &self.fields.bounds;
        if !usage.valid()
            || usage.input as u64 > bounds.input
            || usage.output as u64 > bounds.output
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    pub(crate) fn bounds(&self) -> (u64, u64) {
        (self.fields.bounds.input, self.fields.bounds.output)
    }

    pub(crate) fn semantics(&self) -> &ReservationSemantics {
        &self.fields.semantics
    }
    // @context-fold end frozen-contract-validation
}

// @context-fold begin reservation-contract-storage
pub(crate) async fn load(
    tx: &mut sqlx::PgConnection,
    charge: &Charge,
) -> Result<Option<ReservationContract>, Error> {
    type Row = (
        Option<serde_json::Value>,
        Option<serde_json::Value>,
        Vec<u8>,
        Vec<u8>,
    );
    let (value, marker, fingerprint, spec): Row = sqlx::query_as(
        "SELECT e.detail,a.detail,r.fingerprint,r.spec_digest \
         FROM core_charging.requests r \
         LEFT JOIN core_charging.usage_events e ON e.request_id=r.id AND e.event_id=$2 \
         LEFT JOIN core_charging.usage_events a ON a.request_id=r.id AND a.event_id=$3 \
         WHERE r.id=$1",
    )
    .bind(&charge.id)
    .bind(EVENT_ID)
    .bind(ADMISSION_EVENT_ID)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::NotFound)?;
    let (value, marker) = match (value, marker) {
        (None, None) => return Ok(None),
        (Some(value), Some(marker)) => (value, marker),
        _ => return Err(Error::Invalid),
    };
    let contract = ReservationContract::from_value(value)?;
    let marker: AdmissionMarker = serde_json::from_value(marker).map_err(|_| Error::Invalid)?;
    contract.validate(charge)?;
    if marker != contract.admission_marker()
        || contract.fields.request_fingerprint != hex_digest(&fingerprint)
        || contract.admission_spec_sha256 != hex_digest(&spec)
    {
        return Err(Error::Conflict);
    }
    Ok(Some(contract))
}

pub(crate) async fn insert(
    tx: &mut sqlx::PgConnection,
    charge: &Charge,
    spec: &str,
    proof: &PreparedReservation,
) -> Result<(), Error> {
    let contract = proof.freeze(charge, spec)?;
    let marker = serde_json::to_value(contract.admission_marker()).map_err(|_| Error::Invalid)?;
    let value = contract.to_value()?;
    sqlx::query(
        "INSERT INTO core_charging.usage_events(request_id,event_id,detail) \
         VALUES ($1,$2,$3),($1,$4,$5)",
    )
    .bind(&charge.id)
    .bind(EVENT_ID)
    .bind(value)
    .bind(ADMISSION_EVENT_ID)
    .bind(marker)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
// @context-fold end reservation-contract-storage

// @context-fold begin prepared-contract-unit-tests
#[cfg(test)]
mod tests {
    use super::*;

    fn prepared() -> PreparedReservation {
        PreparedReservation::context_max(
            "01".repeat(32),
            ReservationBinding {
                route_id: "synthetic-route".into(),
                target_id: 1,
                upstream_id: 2,
                protocol: "responses".into(),
                model_snapshot: "synthetic-model".into(),
                generation_url: "https://synthetic.invalid/v1/responses".into(),
                credential_ref: crate::routing::CredentialRef { id: 3, revision: 4 },
                payload_policy_sha256: "02".repeat(32),
                provider_contract_revision: "synthetic-v1".into(),
                projection_revision: "context_max_v1".into(),
                quote_version: 1,
            },
            "03".repeat(32),
            ReservationBounds {
                model_input_max: 100,
                input: 100,
                requested_output: 8,
                output: 8,
            },
            ReservationSemantics {
                input: "inclusive".into(),
                output: "includes_reasoning".into(),
                missing_cached_tokens_is_zero: true,
                missing_cache_creation_tokens_is_zero: false,
            },
        )
        .unwrap()
    }

    #[test]
    fn prepared_contract_digest_binds_wire_route_and_credential_revision() {
        let original = prepared();
        for variant in 0..3 {
            let mut fields = original.fields.clone();
            match variant {
                0 => fields.wire_sha256 = "04".repeat(32),
                1 => fields.binding.route_id = "other-route".into(),
                _ => fields.binding.credential_ref.revision += 1,
            }
            let changed = PreparedReservation::context_max(
                fields.request_fingerprint,
                fields.binding,
                fields.wire_sha256,
                fields.bounds,
                fields.semantics,
            )
            .unwrap();
            assert_ne!(changed.digest(), original.digest());
        }
        assert!(
            original
                .verify_basis(ReservationBasis::Tokens {
                    input: 99,
                    output: 8
                })
                .is_err()
        );
        assert!(
            original
                .verify_basis(ReservationBasis::Tokens {
                    input: 100,
                    output: 8
                })
                .is_ok()
        );
    }

    #[test]
    fn prepared_contract_requires_explicit_null_and_rejects_changed_digest() {
        let proof = prepared();
        let mut value = serde_json::to_value(&proof.fields).unwrap();
        value["prepared_sha256"] = proof.prepared_sha256.clone().into();
        value["admission_spec_sha256"] = "04".repeat(32).into();
        value["frozen_price_sha256"] = "05".repeat(32).into();
        assert!(ReservationContract::from_value(value.clone()).is_ok());
        value.as_object_mut().unwrap().remove("counter");
        assert!(ReservationContract::from_value(value).is_err());
        let mut corrupted = proof;
        corrupted.fields.bounds.output += 1;
        assert!(corrupted.validate().is_err());
    }

    #[test]
    fn prepared_contract_systemone_preserves_input_only_pricing() {
        let source = prepared();
        let mut binding = source.fields.binding.clone();
        binding.protocol = "systemone".into();
        let native = PreparedReservation::systemone_input_only(
            source.fields.request_fingerprint,
            binding,
            source.fields.wire_sha256,
            100,
            source.fields.semantics,
        )
        .unwrap();
        let basis = ReservationBasis::Tokens {
            input: 100,
            output: 0,
        };
        let mut request = super::super::Request {
            id: "native-input-only".into(),
            fingerprint: [1; 32],
            price: super::super::Price {
                user_trust: None,
                image: None,
                upstream_cost_group_ppm: None,
                tiered: None,
                version: "v1".into(),
                model: "jev-native".into(),
                group: "allowed".into(),
                input_per_million: 1_000_000,
                cached_per_million: 0,
                output_per_million: 0,
            },
            reserve: 100,
            lease_seconds: 60,
            maximum_seconds: 60,
        };
        assert!(native.verify_request(&request, basis).is_ok());
        assert_eq!(native.fields.bounds.requested_output, 0);
        assert_eq!(native.fields.bounds.output, u64::from(u32::MAX));
        assert_eq!(
            native.fields.semantics.output,
            "unpriced_systemone_metadata"
        );
        assert!(
            native
                .verify_basis(ReservationBasis::Tokens {
                    input: 100,
                    output: 1
                })
                .is_err()
        );
        // A native receipt can contain many output tokens; allowing this mode
        // with even a positive fractional output price would under-reserve.
        request.price.output_per_million = 1;
        assert!(native.verify_request(&request, basis).is_err());
    }
}
// @context-fold end prepared-contract-unit-tests
