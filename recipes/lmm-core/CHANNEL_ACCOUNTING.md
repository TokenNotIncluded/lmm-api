# Channel balances, consumption and copies

Core owns channel identity and publishes configuration, independent encrypted
credentials and display snapshots in the same audited transaction. Deleting a
channel retires its ID permanently. Native admission also reserves primary,
fallback and image channel IDs inside its existing reservation transaction,
before funding writes. An unfunded admission rolls these metadata writes back;
later settlement or refund of an admitted request remains available after retirement. An explicitly requested previously used ID
is rejected; generated copies allocate above every recorded ID. Sensitive
configuration reads expose the read-only `channel_id_high_water` hint so the
create form proposes a free ID after deletions. The hint is not publication input
and never substitutes for the transactional uniqueness check.

## Consumption

`GET /core/v1/admin/routing/channels/{id}/usage` requires the current
administrator Session and ChannelRead permission. Optional `start_timestamp`
and `end_timestamp` are Unix seconds; the default and maximum window is 31 days.
The Native projection validates frozen request provenance, actual dispatch,
settlement commands and refunds in one SQL snapshot. A failover is attributed to
the dispatched upstream, not today's routing configuration. Holds and uncertain
requests are reported separately from settled consumption.

`GET /core/v1/admin/routing/channels/{id}/counters` also returns lifetime Native
consumption, an immutable inherited display baseline, their confirmed subtotal,
and the cached provider balance. Window consumption excludes inherited baselines.
Credit amounts and counts are decimal strings; 500000 Native credits equal one
USD. Provider balances retain their configured unit and are never converted into
credits, tokens or supplier cost.

`historical_coverage=false` preserves the missing historical opening. In that
case `used_quota` is null and the combined amount is only the confirmed subtotal.
Missing provenance and a reused historical ID prevent a completeness claim.
The current scan bounds requests and receipts and explicitly rejects overflow;
it does not return a successful truncated total. This is not historical channel
opening import, provider-cost accounting or complete legacy statistics parity.

## Provider balance

`PUT /core/v1/admin/routing/channels/{id}/balance-config` takes
`{expected_version,probe}` under the existing sensitive-write permission.
`probe` is null or `{path,json_pointer,unit_label}`: a same-origin relative GET
path, an RFC 6901 JSON pointer and an optional unit label. This explicit plan is
not automatic provider-specific balance discovery.

`GET /core/v1/admin/routing/channels/{id}/balance` requires ChannelOperate and
uses the channel's existing server-side credential. The request uses the normal
gateway's endpoint, timeout and body limits. No database transaction or actor
lock spans network I/O. The second authorization and publication fence reject a
stale configuration or key before updating the observation and audit atomically.
Failure leaves the previous observation intact. The legacy
`GET /api/channel/update_balance/{id}` retains its numeric `data` and adds the
typed observation. An absent configured probe fails explicitly.

Native observations carry a nullable round-trippable balance string, unit,
server observation time, kind, source channel and record time. Missing values and
times remain unknown. A real zero remains visible. Neither the browser clock nor
a copied/reset snapshot represents a fresh provider observation.

## Copy semantics

Native `POST /core/v1/admin/routing/channels/{id}/copy` accepts
`{expected_version,suffix,reset_balance?}`; the flag defaults to true. Legacy
`POST /api/channel/copy/{id}?suffix=...&reset_balance=...` retains its default
`_复制` suffix and explicit empty suffix. Copy always creates an independently
encrypted credential, published routing bindings and metadata.

With `reset_balance=true`, the new display balance and consumption start at
zero. With false, Core captures the source's cached balance and confirmed gross
and refunded display counters. Both retain the original balance observation
time, including an unknown time; the snapshot kind records reset or copy.
Unknown historical coverage remains unknown. Later charges, refunds or balance
refreshes on either channel do not rewrite the other's inherited snapshot.

For older managed Core versions, the Web default copy omits the new optional
flag. Preserve-copy and new balance/statistics actions require fresh, explicit
backend capabilities; absent capabilities never imply support.

Neither mode creates a charge, refund, wallet entry or subscription usage.
Global financial statistics continue reading the original ledger once. Failed
CAS, audit or publication rolls back the new ID, baseline, observation and key.
An unknown commit result retains the existing explicit recovery behavior and
must not trigger an automatic second copy.

## Installation and evidence

Component 43 is installed only by explicit fresh initialization or the exact
offline upgrade. `core-channel-accounting-v1` accepts the frozen 42-component
PostgreSQL 18 predecessor. The existing frozen 32 and 41 predecessors retain
their explicit procedures targeting the current 43-component manifest. Runtime
startup performs no DDL and cannot rewrite inherited baselines, delete identities
or access owner-only upgrade receipts.

The change retains the full completion contract. Local synthetic database/API
checks, Web tests/build, independent acceptance, browser rendering, historical
migration and production deployment are separate evidence. Tests are not proof
that all old provider adapters, historical channel openings or production
cutover have passed.
