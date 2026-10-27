# Legacy pending-liability reconciliation

`scripts/preview-legacy-pending-liabilities.py` is a read-only validator for
the frozen legacy export. It verifies the archive, financial export and
identity bindings, preserves the original rows, and emits a preview with
`settlement_state=unknown`, `spendable=false`, and zero ledger writes. It does
not connect to PostgreSQL, contact a payment provider, grant credit, refund an
order, or close a liability.

The current ArchDmit snapshot contains 82 unresolved payment rows: 37 pending
top-ups and 45 pending subscription orders. A source `pending` status is not a
provider receipt. Provider status, callback delivery and external payment are
still unobserved, so the validator reports
`BLOCKED_PENDING_PROVIDER_RECONCILIATION`.

The schema has an immutable
`core_billing.legacy_pending_liability_receipts` table, but it does not yet
have a safe append function. Direct table writes are revoked. A future writer
must be designed and independently tested before any receipt is appended. The
contract must include all of the following in the request and durable row:

- exact migration, source table/key, source row hash, archive hash and mapping
  hash, checked against the immutable parent liability;
- provider, merchant, environment, store/product and order or transaction
  binding;
- raw amount, currency and unit, preserved without conversion or invented
  values;
- evidence digest, signed payload/proof and an explicit signature-verification
  result;
- an allowlisted provider status and settlement state, with `unknown`,
  `unresolved`, `manual_review`, missing proof, or unsigned evidence unable to
  reach a terminal state;
- exact-key replay returning the original result, conflicting replay rejected,
  serializable durable commit, and `spendable=false`/`spendable_units=0` for
  every receipt.

The function must accept calls only from the offline schema owner or a
separately reviewed maintenance role. The runtime role may read the source and
receipt tables but must not insert, update, delete, truncate, or execute the
writer. Appending a provider receipt must not mutate the source liability or
post a wallet, reserved, debt, affiliate, refund, or ledger entry. Terminal
payment settlement requires a separate reviewed Core payment proof and ledger
contract; a receipt alone cannot authorize credit.

Until provider-specific verification contracts exist for ePay and
Waffo Pancake, keep every unresolved row fenced. Run the validator again after
the final writer fence and a fresh frozen capture; any archive, mapping,
source-row, target, or provider-evidence drift requires a new review.
