# Legacy provider verification contract

This is a source-only contract for reviewing the pending legacy rows. It does
not contact ePay, FastPay or Waffo Pancake, verify a signature, append a
receipt, close a liability, or post a ledger entry. Every result remains
`verification_status=UNVERIFIED`, `settlement_state=unknown`,
`spendable=false`, and `spendable_units=0`.

The executable shape checker is
[`scripts/legacy_provider_contract.py`](../scripts/legacy_provider_contract.py).
It accepts only hash-bound evidence metadata. It rejects raw provider payloads,
signatures and unknown fields, so an evidence fixture cannot quietly become a
credential or replay source.

## Provider-specific fields

| Provider | Source fields required before review | Evidence bindings required | Current state |
| --- | --- | --- | --- |
| ePay | `trade_no`, explicit raw amount and explicit currency | merchant hash, environment, CNY currency, raw amount hash, transaction hash, event hash, payload hash, signature hash, provider status and observation time | The legacy adapter is Go-only; Core has no ePay proof verifier. A CNY assumption cannot replace a missing source currency. |
| FastPay | No reviewed source/provider contract exists | No generic evidence is accepted | Explicitly unsupported and fenced. |
| Waffo Pancake | `trade_no`, provider transaction/event IDs, store ID, product ID and settlement currency | merchant/environment, amount and currency, transaction/event/store/product hashes, payload/signature hashes, provider status and observation time | The old monolith has a callback verifier, but the new Extensions/Core boundary has no approved verifier or receipt contract. |

The checker compares hashes to the frozen source row and raw amount/currency
objects where the source provides them. A shape-valid result means only that a
future independently reviewed verifier has enough fields to inspect; it is not
a provider attestation.

The current 82-row snapshot therefore remains fenced. Missing provider
transaction IDs, event IDs, store/product bindings, currency facts or signed
payload evidence stay in `INCOMPLETE_PROVIDER_EVIDENCE`. No row is promoted to
`paid`, `failed`, `refunded` or any other terminal settlement state.

Future work must add provider-specific verification implementations, an
owner-only append-only receipt function with amount/currency/order/merchant/
environment/account/signature bindings, and independent tests against each
provider's real contract before any terminal receipt can be considered.
