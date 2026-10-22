# Native price and priority administration

This module serves the actual Rust routing release. Price import/export, per-model
FX selection, financial review and channel priority changes use the native
administrator Session, current MFA/policy checks and an atomic publication.
The Web entry points are **Model settings → Native price synchronization** and
**Channels → Native priority ordering**. The channel dialog has its own native
catalog request and does not require the legacy Go channel list to load.

## Establish the first durable release offline

A fresh installation may serve its reviewed file configuration, but the network
runtime cannot register that configuration as database authority. Before using
online publication, an installation owner must explicitly establish the baseline:

```sh
lmm-core --config /private/installer-owner.toml configure-routing-baseline < /private/release.json
```

Input is the same bounded `ReleaseDocument` JSON used for the model routing file:
`contract_version`, `routing` and `pricing`. The limit is 16 MiB. The command
validates and compiles the complete document, checks the installed schema receipts,
then verifies that the connection is the actual owner of the routing and receipt
tables. It registers credential **references**, stages prices and publishes the
initial release with an expected absent active pointer in one synchronous
transaction. No credential secret is accepted or printed. The operator must have
already installed each referenced secret revision through the existing host secret
configuration. The JSON receipt contains only versions, snapshot SHA-256 and whether
the baseline was newly created.

Repeating an identical document returns `created: false`. A different active
release is a conflict, including after an online price or priority publication.
The command never replaces or resets a durable release. Do not run it as the
network runtime role, and do not use `--legacy-env`.

Both SQL preview and publication wrappers reject a missing durable active release
with `P2703`; they never establish a baseline from a runtime-supplied `before`
document. With a baseline, the complete `before` document must equal the durable
active snapshot. Immutable destinations, credential references, groups, models,
protocols and metering semantics therefore remain anchored in owner-approved data.

## Online publication and permissions

The runtime installer grants schema USAGE and exactly five wrappers: read-active,
store/read-preview, read-operation and publish-preview. Direct access to preview or
operation tables, helper execution, sequence access and grant options are forbidden
by the read-only startup ACL checker. A real administrator is reauthorized inside
the definer wrappers. A read snapshot alone never grants mutation authority.

Previews expire after five minutes and are bounded to four per administrator and
64 globally. Confirmation binds the complete source/FX evidence, expected versions,
full current/source/result prices and snapshot digest. Publication uses CAS and a
stable operation ID. Notification insertion shares its transaction. Undo creates
another preview and increasing release version rather than rewriting history.

The current price source format is `core-native-price-source.v1`, in USD. Each
selected model retains its model/group/speed identity, tier boundaries and provider
metering contract; only exact monetary coefficients change. Unrepresentable scalar
amounts fail instead of rounding. `USDExchangeRate` is read from current site options
without a fallback. Selecting keep needs no exchange rate. Multiply/divide rechecks
the exact option value and revision at publication.

After a durable publication each node refreshes before accepting new model work.
Price and OAuth catalog reads refresh in their existing read transaction. A missing
or incompatible durable release fails closed, and in-flight work retains its frozen
price. Restarting never silently replaces published prices with old file prices.

## Installation and validation status

Component 27 introduces the private storage and wrappers, explicitly qualifies the
five existing routing functions' public table references under `pg_catalog`, and
extends the durable protocol CHECK to the seven protocols understood by the Rust
compiler. It does not broaden runtime table writes.

The existing-install 26→27 upgrade remains unaccepted until the exact predecessor
catalog fingerprint is independently observed on PostgreSQL 18. The evidence test
prints its predecessor manifest/source hashes and actual fingerprint, then fails
with `PIN_PENDING`. Do not treat that probe or formatting as successful migration
validation. See `check-results/native-routing-admin.md` for actual checks and gates.
