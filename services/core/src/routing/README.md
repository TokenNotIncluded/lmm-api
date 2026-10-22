# Rust routing and pricing — contract v1

Task 04, based on `b83723cdac3f148140806b8025cb5360a2f6a26d` of
`wip/rust-core-go-extensions` / parent PR #675. Working branch: `wip/mk-04-routing`.
This is a routing component, not a claim that the model HTTP API is ready.

## Ownership and integration

The module owns route selection, versioned prices, health gates, and the retry plan.
It makes no Go calls, forwards no HTTP requests, and writes no ledger entries.
Normal request selection uses memory only. PostgreSQL is used by Rust to publish
configuration and recover after restart, never for each routing decision.

Task 05 (`wip/mk-05-relay`) owns HTTP clients, credential resolution, protocol
conversion, streaming, cancellation, and reporting transport outcomes. Task 06
(`wip/mk-06-protocol-events`) owns Protobuf messages and field numbers. Neither a
new proto file nor a shared `lib.rs`, `main.rs`, HTTP entry point, or Go file is
changed here. `tests/routing.rs` compiles the module independently until the
integration owner adds `pub mod routing` to the core library. Once integrated,
remove that bridge so module unit tests are not compiled a second time.

`CONTRACT_VERSION = 1` is the Rust contract. It is NOT a Protobuf field number.

## Task 05 call sequence

1. Get an authenticated group from identity/billing. Do not trust a raw client
   group field. Routing does not grant group membership or choose the payer.
2. Call `Router::route(RouteRequest)` exactly once for the logical request.
3. Keep the returned `RoutePlan`, and reserve funds using its `PriceQuote`.
4. Call `plan.start()` for the first `Attempt`. Resolve `CredentialRef { id,
   revision }` through the Rust secret service, not through Go. The revision is
   immutable; never fall back to a different key when it is unavailable.
5. Use the endpoint, protocol, approved upstream model, payload policy and timeout
   from the attempt. Check `attempt.is_available()` immediately before sending.
6. On failure call `plan.retry(Failure)`, not `Router::route` again. Record the
   attempt number and route/target IDs. Keep upstream error bodies out of logs.
7. Settle one logical request with the pinned quote and normalized final usage.
   A retry is not a new per-request fee. Task 03 decides which incurred usage is
   chargeable and enforces reservation/settlement idempotency.

Core types and methods:

```rust,ignore
let mut plan = router.route(RouteRequest {
    model: "model-a",
    group: authorized_group,
    speed: Some(Speed::Fast),
    protocol: Protocol::OpenAiChat,
    replay: ReplayPermission::Denied,
})?;
let pinned_price = plan.quote();
let first = plan.start()?;
// Task 05 sends first, using its private credential resolver and HTTP client.
let second = plan.retry(Failure {
    kind: FailureKind::Connect,
    delivery: DeliveryState::NotSent,
})?;
assert_eq!(first.version(), second.version());
assert_eq!(first.quote(), second.quote());
```

Each attempt exposes `version`, `quote`, `number`, `speed`, `route_id`, `target_id`,
`upstream_id`, `canonical_model`, `group`, `endpoint`, `upstream_model`, `protocol`,
`payload_policy`, `credential_ref`, and `timeout_ms` as read-only methods. Private
fields prevent accidental replacement of a selected destination.

`PayloadPolicy::Passthrough` requires the exact same model name and wire protocol.
Aliases, model-name suffixes, or protocol changes require an explicit `Adapt`
target. The adapter may translate the approved model name/protocol; it must not
remove reasoning controls, change sampling, or substitute another canonical
model. The routing component itself never changes a body or inserts a service
level parameter. Task 05 must still enforce outbound host/IP policy, TLS, header
filtering, response limits, and redirects; this module is not an HTTP firewall.

## Group, speed and selection rules

A group is matched exactly. An alias belongs to one group and points directly to
one canonical route in that group. Duplicate names, shadowing and alias chains
are rejected. There is no cross-group or cross-model fallback.

`standard`, `fast`, and `ultrafast` must be explicitly allowed by the route, target,
and matching price rule. Missing fast/ultrafast configuration is an error, not a
standard-speed fallback. `model/fast` and `model/ultrafast` are recognized as
suffixes. Conflicting explicit speed and suffix are rejected. Canonical model
names and aliases cannot end with these reserved suffixes. For unchanged request
bodies, prefer a `/fast` or `/ultrafast` HTTP path mapped by task 05 to the explicit
speed field rather than adding a suffix to the JSON model field.

Smaller priority wins. Within a priority tier, selection is weighted random with
an unbiased integer draw using the existing `rand` dependency. Fallbacks are
weighted draws without replacement; only after a tier is exhausted is a lower
priority considered. Each route can have at most 64 targets, one target per
upstream, and at most 8 total attempts. A zero weight, disabled target, disabled
upstream, unhealthy/unknown state, incompatible passthrough, or unsupported speed
excludes a candidate. All unavailable means an error, never a fallback model.

The complete ordered candidate list is pinned at request start. Live health and
operational changes may remove candidates later, but cannot add a new target.
The candidate count is bounded by the configured attempt limit. A candidate
removed after planning is not replaced by a new lookup.

## Failure and replay policy

The default policy is one attempt and no retries. A configured retry cause may be
`Connect`, `Timeout`, `RateLimited`, or `ServerError`. Authentication failures,
client errors, invalid responses, and cancellation always stop the plan.

`NotSent` means no request bytes reached the provider. `Rejected` means the
provider is known not to have accepted work; a status code alone is not always
proof. A failure that might have been accepted is `PossiblyAccepted`. Retrying it
requires BOTH configuration permission and explicit request replay permission.
Task 05 must not invent idempotency support. `ResponseStarted` means response body
bytes were exposed to the client; retry is always prohibited, even when both
permissions are set. A denied retry ends the plan permanently.

Health is a Rust-owned runtime concern, separate from immutable pricing. New
destinations start optimistically healthy; a Rust probe can set them to `Unknown`
until tested. This module does not start a probe worker or infer a circuit-breaker
timeout. Task 05 may call `Attempt::mark_unhealthy` after an infrastructure failure.
A trusted Rust health monitor reopens destinations using `Router::set_health`
with the current release version. Production integration must supply that monitor.
An old credential/endpoint failure does not poison its replacement. Route/price-
only updates retain health and emergency stop state for unchanged destinations.

A configuration disable affects NEW requests, consistent with pinned plans.
For an immediate operational stop, use `set_operational_enabled`; it also blocks
not-yet-sent attempts in existing plans. Health reports do not clear that flag.
It cannot revoke bytes already sent to an upstream.

## Money and price evidence

`TokenRates` use integer **micro-USD per 1,000,000 tokens**. The `request` rate is
micro-USD per logical request. One micro-USD is USD 0.000001. There is no conversion
through the old Go quota constants. Group and speed multipliers use parts per
million: `1_000_000 = 1x`, `2_000_000 = 2x`. Each multiplier must be positive and no
larger than `1_000_000_000` (1000x).

`Usage.input`, `output`, `cache_read` and `cache_write` are DISJOINT. Task 05 must
subtract included cache tokens from a provider's total input count before passing
usage to this module. Providers whose cache counters overlap need normalization,
not direct addition. The quote sums all dimensions and the single request fee,
applies both multipliers in checked `u128`, and rounds UP once to micro-USD. A
result beyond PostgreSQL signed `bigint` or any intermediate overflow is an error.
No saturation, floating point, or silent overflow is allowed.

Task 03 should persist the routing version, price version, price rule ID, both
multipliers and normalized usage with its request accounting evidence. No ledger
or billing mutation is implemented by this module.

### Decimal token tariffs

`billing_mode = "tiered_decimal"` carries an exact rational tariff, independently
of the integer `TokenRates` placeholders. Revision 1 keeps its text/cache-read
contract and rejects cache creation. Revision 2 adds the source `cc` price through
`cache_write`, and an optional `cache_write_1h` rate with the matching
`uses_cache_write_1h` flag. A referenced zero-price category still changes prompt
normalization; removing `cc * 0` from an imported expression changes its meaning.

The inclusive input total chooses the context tier. In revision 2, for inclusive
APIs, `cc` is the aggregate creation count and `cc1h` is zero. For Anthropic, prompt input
excludes cache reads and all creation; `cc` and `cc1h` are distinct five-minute and
one-hour counts. A known aggregate plus either TTL determines the other by exact
subtraction. A positive aggregate without either TTL remains unknown. Inconsistent
or overlapping disjoint counters are rejected rather than clamped into a price.

Revision 2 freezes the selected provider's reviewed
`missing_cache_creation_tokens_is_zero` policy in the persisted quote as
`missing_cache_creation_is_zero`. It defaults to false, is independent of the
cache-read absence policy, and cannot come from an HTTP request or a model name.
Recovery uses that original policy, rates and input semantics. It does not read
today's provider policy or use supplier-reported cost as the user's tariff.

Revision 3 retains the existing flat `generic_remainder_v1` contract: the effective
creation total is the larger of the reported aggregate and the sum of known
five-minute/one-hour counts. Unclassified creation remains a distinct remainder;
missing TTLs are not changed into measured zeroes. It does not enable OpenAI's
twelve-hour price or reinterpret frozen revision 1/2 quotes.

All supported decimal revisions sum exact terms and round the final credit amount
half upwards with the positive one-credit minimum. Reservation rounds upwards and
considers every tier and every cache split, including the supported TTL prices.
These rules do not claim bit equivalence with legacy intermediate floating-point
arithmetic. Media, calendar/request expressions and tool surcharges need their
own supported contracts; adding cache creation does not enable them.

#### OpenAI twelve-hour cache writes (revision 4)

Revision 4 is an explicitly configured tariff for inclusive OpenAI Chat/Responses
usage. It requires `uses_cache_read`, `uses_cache_write` and
`uses_cache_write_12h`, forbids `uses_cache_write_1h`, and requires an explicit
`cache_write_12h` rate in every tier. A reviewed zero rate is valid; an absent rate
is not a default. Revisions 1–3 retain their original accounting rules and reject
nonzero twelve-hour usage rather than apply a new price to an old quote.

Counts come from the supplier's usage receipt: Chat uses `prompt_tokens_details`
and Responses uses `input_tokens_details`. Each contains `cache_write_tokens`
and, when reported, `cache_write_tokens_details.ttl_30m_tokens` and
`cache_write_tokens_details.ttl_12h_tokens`. The aggregate write count includes
both lifetimes. For total input `I`, cached reads `R`, aggregate writes `C` and
measured twelve-hour writes `H`, the disjoint pricing quantities are:

| Rate | Token quantity |
| --- | --- |
| `input` | `I - R - C` |
| `cache_read` | `R` |
| `cache_write` (thirty-minute writes) | `C - H` |
| `cache_write_12h` | `H` |
| `output` | The reported output count |

The inclusive `I` selects the context tier. A reported thirty-minute count must
agree with the partition above; known buckets cannot exceed the aggregate, and
two reported buckets must sum to it. Negative, fractional, overflowing,
contradictory or decreasing cumulative evidence is rejected. Positive writes
without a measured twelve-hour count leave settlement pending and retain the
hold; absence is not zero. An explicit zero twelve-hour count is sufficient to
price a known positive aggregate at the ordinary write rate. Request TTL options
never supply missing usage evidence.

The following is a complete illustrative `PriceRule`, suitable as an entry in
`PriceConfig.rules`. Its rates are **examples, not official prices or automatic
defaults**. `DecimalRate` is exact USD per million tokens. Replace the synthetic
`source_digest` with the SHA-256 of the administrator's approved source expression
and use the site's authorized model/group and price version. The outer integer
`rates` are zero placeholders for `tiered_decimal`.

```json
{
  "id": 1,
  "model": "example-cache-model",
  "group": "default",
  "speed": "standard",
  "billing_mode": "tiered_decimal",
  "rates": { "input": 0, "output": 0, "cache_read": 0, "cache_write": 0, "request": 0 },
  "group_multiplier_ppm": 1000000,
  "speed_multiplier_ppm": 1000000,
  "tiered": {
    "revision": 4,
    "source_digest": "4545454545454545454545454545454545454545454545454545454545454545",
    "uses_cache_read": true,
    "uses_cache_write": true,
    "uses_cache_write_1h": false,
    "uses_cache_write_12h": true,
    "tiers": [
      {
        "up_to_input_tokens": null,
        "rates": {
          "input": { "numerator": 2, "denominator": 1 },
          "output": { "numerator": 10, "denominator": 1 },
          "cache_read": { "numerator": 1, "denominator": 5 },
          "cache_write": { "numerator": 5, "denominator": 2 },
          "cache_write_12h": { "numerator": 5, "denominator": 1 }
        }
      }
    ]
  }
}
```

For `I = 100`, `R = 20`, `C = 60`, `H = 40` and five output tokens, this example
prices 20 ordinary input, 20 reads, 20 thirty-minute writes and 40 twelve-hour
writes. The total is USD 0.000344, or 172 credits at 500,000 credits/USD before
any funding discount. Settlement applies the frozen group/speed multipliers and
applicable exact funding discount to the combined rational amount, then rounds
half up once, with the positive one-credit minimum. It does not round each token
category separately. Reservation takes the ceiling of a conservative bound over
every reachable tier and cache split, including twelve-hour writes.

Only the Responses request surface accepts bounded `prompt_cache_options`:
`ttl` is required and must be `"30m"` or `"12h"`; `mode` may be omitted or equal
`"implicit"`. For example:

```json
{
  "model": "example-cache-model",
  "input": "hello",
  "prompt_cache_options": { "mode": "implicit", "ttl": "12h" }
}
```

Explicit cache breakpoints, prewarming and unknown option keys are unsupported.
Core rejects these options before an upstream send unless the selected route is
OpenAI Responses with a revision 4 tariff. Chat can report measured twelve-hour
usage, but these request options are not forwarded through Chat or a protocol
conversion.

The new optional counters and frozen rates use existing JSON evidence/price
storage; this change requires no DDL and no startup schema upgrade. New readers
accept historical JSON with absent fields and preserve the original quote's
revision, rates, usage semantics and absence policy during recovery. Deploy
compatible readers before enabling revision 4. An older binary with
`deny_unknown_fields` cannot safely read the new price/evidence fields; publishing
an older active tariff does not remove frozen requests or historical evidence.
A rollback must retain a compatible reader for recovery and settlement. Never
erase twelve-hour evidence or rewrite a frozen quote to make an older binary load.

Protocol references are the
[QuantumNous/new-api upstream change e9d4d635](https://github.com/QuantumNous/new-api/commit/e9d4d6353ce2c11df753d751ba1ad778e2433a88)
and the [official OpenAI prompt-caching guide](https://developers.openai.com/api/docs/guides/prompt-caching).
This is a Rust implementation of the receipt and tariff contract; the project's
[AGPL attribution and upstream provenance](../../../../docs/fork.md) remain in
force. Search has its own measured-unit contract described under
[Hosted search receipts and prices](#hosted-search-receipts-and-prices).

## Publication, persistence and recovery

The offline initializer must explicitly install `schema/routing.sql` in its fresh
schema transaction. This change does not register the schema in the shared
initializer and never upgrades a database at startup.

`PostgresRoutingStore::register_credential_reference` is for the Rust secret manager
after it durably writes a secret revision. It registers numeric IDs only; it does
not store or verify secret material. A route referring to an unregistered pair
fails its database foreign key. Resolve failures still fail closed at send time.

`stage_prices(PriceConfig)` validates and seals an independent price version without
moving traffic. `publish(RoutingConfig, price_version, expected_version)` validates
all route/price combinations, creates the immutable artifact and normalized
projections, seals them, and moves the active pointer in one database operation.
A row lock and expected version prevent lost updates. Reusing a price version with
different stored JSON is rejected. Existing artifacts/projections cannot be
updated, deleted, or appended after sealing. Normalized tables are generated from
the artifact inside the publication function, not maintained by Go.

The active pointer references one routing version, which references one price
version. `load_active()` reads that pair in one SQL statement. A recovered
`ReleaseDocument::into_router()` is sufficient to route without Go. A failure to
load a new release must keep the last good in-memory release; startup without a
valid release must report not ready. Do not silently create default prices.

After commit, task 06 should deliver the version notice to Rust nodes; each node
loads the active document and calls `ReleaseDocument::apply`. Delivery must be
retryable, and a periodic Rust reconciliation read should cover missed notices.
The durable commit and per-process install are NOT a distributed atomic commit.
Nodes can temporarily serve different complete versions, never a mixed pair.
Requests pin their own pair. On a local publication conflict, reload the durable
active version. An already applied notice should be acknowledged by comparing
both version numbers before calling `apply` again. Rollback uses a higher version
with previously approved content, never a decreasing pointer.

Runtime publication validates outside a short write lock, checks the expected
version again, then swaps one `Arc<Snapshot>`. Requests release the read lock before
route selection and hold no lock across network I/O. Retired snapshots live until
the last request releases them. This is not a claim of lock-free operation.

Only `Router::catalog(authorized_group)` is a public projection. It omits upstream
addresses, IDs, credentials and raw tuning documents. Release documents are
internal control-plane records. Errors retain no SQL or upstream diagnostic text.
`Attempt`, `Endpoint`, and `CredentialRef` have redacted debug output; credential
values have no field in the route configuration or decision types.

## Validation and benchmark

The integration test bridge includes pure routing tests and isolated PostgreSQL
tests. No live model provider or paid API is used. With the pinned Rust toolchain:

```sh
cd services/core
rustfmt --edition 2024 --check src/routing/mod.rs tests/routing.rs
cargo clippy --locked --test routing -- -D warnings
# Mock/in-memory tests only, without PostgreSQL:
cargo test --locked --test routing -- --skip postgres_
# Full tests: DATABASE_URL must point to a DISPOSABLE PostgreSQL server.
# SQLx creates separate test databases; do not use production credentials.
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres \
  cargo test --locked --test routing -- --nocapture
```

The dedicated [`Rust routing contract` workflow](../../../../.github/workflows/core-routing.yml)
runs these component checks only when manually dispatched. It has no production
or deployment access and does not run the model HTTP/ledger integration suite.

Revision 4 additionally needs the receipt, pricing, recovery and request-option
unit tests and the actual PostgreSQL settlement tests. From `services/core`, the
focused commands are:

```sh
cargo test --locked --lib openai_
cargo test --locked --lib cache_write_12h_projection_tests
cargo test --locked --lib responses_cache_lifetime
cargo test --locked --lib cache_12h_replay
# DATABASE_URL must already point to a disposable PostgreSQL server.
cargo test --locked --test tiered_token_billing
```

The root `bun run test:core` script runs `cargo test --locked --all-targets`;
the broader [`core-protocol` workflow](../../../../.github/workflows/core-protocol.yml)
also compiles/lints all targets and runs that aggregate with PostgreSQL and its
process fixtures. Reuse an aggregate result instead of repeating its component
checks. The local implementation checks for revision 4 included the pinned
Rust 1.99.0 formatter, but did **not** include Rust compilation/tests or PostgreSQL
settlement tests. Subsequent Cargo lockfile resolution and metadata validation
also do not compile the implementation. Source tests and format checks are not
financial acceptance or proof of live-provider compatibility; consult the
revision's actual `check-results` and CI records before enabling it.

The ignored test below is an opt-in pressure benchmark, not a skipped correctness
test. It uses configured model cardinality, two weighted mock destinations per
model, concurrent workers, pre-measurement warmup and a start barrier.

```sh
cd services/core
rustc -Vv
ROUTING_BENCH_REQUESTS=1000000 ROUTING_BENCH_THREADS=4 ROUTING_BENCH_MODELS=1000 \
  cargo test --release --locked --test routing routing_benchmark -- --ignored --nocapture
```

Record the commit, compiler, CPU, available memory, OS, thread count, model count,
and full output. Repeat with 1/2/4/8 threads and larger catalogs on the SAME machine.
The output measures route + first-attempt construction only: wall time, throughput,
and one-in-100 sampled p50/p95/p99 latency. It includes RNG/allocation costs but no
HTTP, database calls, tokenization, or settlement. Sampling/timing overhead is
included, and the fixed every-100 sampling pattern is not a full latency census.
Compare like-for-like runs; do not present these numbers as end-to-end API capacity.
No benchmark result is claimed until an actual run has completed.

## OpenRouter account-cost routers

The Rust price configuration accepts `billing_mode: "upstream_cost"` for
`nvidia/switchyard`, `openrouter/auto-beta`, `openrouter/fusion` and
`typesafe/jev-router`. Omitted mode remains `tokens`. This is a core-owned price
contract; it cannot be selected by client JSON or an extension RPC. For example,
the price rule for an already authorized route/group can be:

```json
{
  "id": 1,
  "model": "openrouter/fusion",
  "group": "default",
  "speed": "standard",
  "billing_mode": "upstream_cost",
  "rates": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0, "request": 0},
  "group_multiplier_ppm": 1000000,
  "speed_multiplier_ppm": 1000000
}
```

All fixed rates must be zero in this mode, the speed must be standard, and the
upstream model must be the same router. A token-only caller of `PriceQuote::charge`
gets an error, rather than treating these zero rates as a free model.

The gateway freezes the rule version and group multiplier, reserves USD 1 times
that multiplier, then settles `ceil(usage.cost USD × 500000 × group multiplier ×
account discount)` credits. The account discount is the frozen, verified factor;
it is applied once. `cost_details.upstream_inference_cost`, final selected-model
rates and local tool surcharges are not added. Decimal JSON numbers are parsed
exactly to 10^-18 USD and rounded up once at the final credit amount; invalid,
negative, excessive or inexact values are rejected. Repeated cumulative SSE
reports replace the prior observation, rather than being summed.

Missing cost after dispatch leaves the reservation pending for reconciliation;
explicit zero refunds the unused hold. A larger measured cost first attempts an
authorized reservation increase against the original payer and budget. Failure
to authorize that increase leaves settlement pending. Receipts persist in the
existing append-only charging events and model output checkpoints, including
replay and completed-stream recovery; no schema migration is required.

The native `/core/v1/pricing/public` and `/core/v1/pricing/session` projections
return `token_price: null` and an `upstream_cost_price` descriptor for this mode.
The descriptor identifies `usage.cost`, USD, 500000 credits/USD, final ceiling
rounding, and an exact string numerator/denominator multiplier. Public rates
include the group multiplier; session rates already include the verified account
discount. Clients must not multiply that discount again or infer free token
prices from the zero fixed rates. If independently priced image variants share a
model name, their image APIs and prices remain independent of text availability.

External text targets remain blocked with
`unavailable_reason: "upstream_budget_contract_required"`. A descriptor documents
the calculation; it grants no invocation authority. OpenRouter workspace/key
budgets are soft limits and can be exceeded by dispatched requests. They do not
satisfy the supplier-enforced budget contract required by the gateway. The
system assistant also continues to reject this mode.

The Web pricing cards, table and details display these rules per group. The
`/system-settings/billing/router-pricing` page reads the native session catalog
without the legacy `/api/option` endpoint and generates a JSON price-rule preview.
It does not save rules, change routing permissions or enable external calls.
Merge a reviewed preview into the installation owner's model TOML `prices.rules`
(using the equivalent TOML fields), preserve existing rule IDs, advance
`prices.version` (the startup loader pins this version in the routing snapshot),
and validate the edited document with the Core model configuration loader in an
isolated installation before the reviewed restart.
The preview lists only the session's visible groups; it is not a complete export
of all installed routes or rules. Never replace the entire price configuration
with a single generated rule. No database price writer is implied by this page.

Local protocol/ledger tests do not certify live OpenRouter compatibility, enable
external forwarding, update the old monolith settings page, or deploy anything.
Provider receipts follow OpenRouter's
[usage accounting contract](https://openrouter.ai/docs/cookbook/administration/usage-accounting).

## Hosted search receipts and prices

Hosted search is opt-in on both the native request and its exact supplier/model
contract. A custom function named `web_search` remains a custom function. The
gateway requires a matching `PriceRule.search`, reviewed finite supplier limits,
and a native protocol route before reserving funds or contacting the supplier.
Unsupported cross-protocol hosted-tool conversions are rejected before dispatch.

`SearchTariff` revision 1 uses exact USD **per one measured unit**, independently
of token prices. Rates imported in CNY or per thousand calls must be converted
explicitly before configuring this USD contract; a numeric value alone does not
establish its denomination. Gemini uses either `query` or `grounded_request` for
one price, never both. xAI can additionally price fetched posts and users as
separate dimensions. Every enabled, priced dimension needs a supplier-backed
finite maximum. A local check after receiving usage is not a spending guarantee.

The following synthetic fields demonstrate the JSON contract, not live prices
or a production supplier spending guarantee:

```json
{
  "search": {
    "revision": 1,
    "provider": "open_ai_responses_v1",
    "unit": "call",
    "currency": "USD",
    "usd_per_unit": {"numerator": 1, "denominator": 1000}
  }
}
```

Merge that object into the reviewed price rule. In the corresponding exact
`ExternalModelContext`, configure an independent supplier contract:

```json
{
  "search": {
    "provider": "open_ai_responses_v1",
    "limits": {"calls": 3},
    "contract_revision": "reviewed-supplier-model-cap-revision"
  }
}
```

The surrounding context pins upstream ID, endpoint, native protocol, mapped
model, input/output ceilings and token absence conventions. Synthetic loopback
search routes also require this explicit binding. OpenAI Responses receives an
enforced `max_tool_calls`; Anthropic receives `max_uses`. Other suppliers rely
on the specifically reviewed finite guarantee in their context. Do not infer
support for OpenAI cap parameters from a compatible response shape.

| Receipt contract | Native protocol | Authoritative search measurement |
| --- | --- | --- |
| `open_ai_responses_v1` | Responses | Unique completed `web_search_call` IDs with `action.type=search`; page opening/finding is preserved without a search charge |
| `azure_responses_v1` | Responses v1 | `tool_usage.web_search.num_requests` |
| `xai_responses_v1` | Responses | `usage.server_side_tool_usage_details.web_search_calls`, `x_posts_fetched`, `x_users_fetched` |
| `dashscope_responses_v1` | Responses | `usage.x_tools.web_search.count` |
| `dashscope_anthropic_v1` | Messages | `usage.server_tool_use.web_search_requests` |
| `anthropic_messages_v1` | Messages | `usage.server_tool_use.web_search_requests` |
| `gemini_grounding_v1` | Gemini generateContent | Unique nonempty trimmed web/image search queries across candidates/frames; grounded-request indicator is zero or one |
| `zhipu_chat_v1` | Chat, nonstreaming | Nonempty search results establish one grounded request; the request flag alone does not |

Repeated cumulative receipts replace previous measurements. Supplier-specific
counters take precedence over output-item estimates. Missing final measurements
are unknown; explicit zero is preserved. Request intent, tool names, citations
and result-array length are never interchangeable with measured billable calls.
Native hosted-tool output and citations are retained in the corresponding
protocol. No raw query set is persisted in the billing checkpoint.

Search and token amounts enter one exact rational expression before the existing
group/speed/verified-account factors and final rounding. Scalar prices round up
once; decimal tiers retain their existing final rounding. Reservations use the
same frozen price and a conservative combined ceiling. `upstream_cost` and image
prices cannot enable this independent search surcharge. Unknown or unsupported
positive usage keeps the original operation pending rather than settling for
zero, replaying the supplier, or obtaining a new price.

Prices and counters use existing immutable price/event/checkpoint JSON. Restart
recovery validates the original provider, unit, limits and evidence; it does not
read current search prices. Enabling these fields requires compatible readers
throughout rollback/recovery. Do not delete new fields from historical receipts.

The native pricing catalog exposes exact string fractions and whether a matching
reviewed route exists. Rates include their documented group/account basis once.
Usage logs and CSV retain measured units and distinguish missing values from
zero. Dashboard cache totals count each settled text aggregate once; TTL subsets
are not added to the write total, and absent measurements propagate unknown.

Development commands with the pinned toolchain and a disposable PostgreSQL:

```sh
cargo test --locked --lib search
cargo test --locked --lib cache_summary
cargo test --locked --test search_billing
```

These test sources include synthetic native suppliers, actual Core admission,
the ledger, duplicate requests and restart recovery. Writing or formatting them
does not establish execution or live-provider acceptance. See this change's
`check-results` records for the exact checks actually run.

Protocol sources: [upstream vendor meters](https://github.com/QuantumNous/new-api/commit/acb347a197ff0e22f651be9813b4a1a867342941),
[upstream Gemini units](https://github.com/QuantumNous/new-api/commit/d10f11ffff20688b8e3e745268cc0036e896d068),
[OpenAI web search](https://developers.openai.com/api/docs/guides/tools-web-search),
and [Google Search grounding](https://ai.google.dev/gemini-api/docs/google-search).

## Native DeepSeek Search and Fetch

The `deepseek_tools` routing protocol serves `POST /v1/tools/search` and
`POST /v1/tools/fetch` as separate nonstreaming operations. The local optional
`model` selects a route and is removed before dispatch; its defaults are
`deepseek-search` and `deepseek-fetch`. These operations use the existing API-key
identity, route selection, reservation, selected-price checkpoint and ledger
settlement path. They do not enter the chat protocol adapter or replay after a
send.

Each operation needs its own matching `PriceRule.search` and exact
`ExternalModelContext.search`: use provider `deep_seek_search_v1` or
`deep_seek_fetch_v1`, unit `call`, and `limits: {"calls": 1}`. The context has
`max_input_tokens: 0` and `max_output_tokens: 0`. All scalar price rates are zero;
`billing_mode` remains `tokens` as the base envelope, with the explicit search
tariff providing the charge. Image, embedding, upstream-cost and tiered prices
cannot be mixed into this native operation. For example, these are fields to
merge into a reviewed Search price rule, using a **synthetic rate, not a supplier
default**:

```json
{
  "billing_mode": "tokens",
  "rates": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0, "request": 0},
  "search": {
    "revision": 1,
    "provider": "deep_seek_search_v1",
    "unit": "call",
    "currency": "USD",
    "usd_per_unit": {"numerator": 1, "denominator": 1000}
  }
}
```

A complete successful Search receipt with nonempty `results` measures one call;
an explicit empty array measures zero. Result count is not the number of billed
calls. Fetch measures one for complete nonempty `content`, and zero for an
explicit empty string. Token counters are explicitly zero for both operations.
Only a complete documented error receipt establishes a failed, free request;
an arbitrary proxy error, timeout, incomplete body or malformed result does not.
Unknown evidence preserves the hold. A verified failed request records zero
usage with a failed terminal outcome, settles the unused hold, and returns a
sanitized gateway error without publishing supplier diagnostic text.

Core limits Search queries to 128 Unicode characters and result `count` to
1 through 50; Fetch URLs are bounded HTTP(S) URLs without embedded credentials.
Unknown request keys and duplicate JSON keys are rejected. Receipt validation
checks the entire supported response shape before deciding a charge or a refund.

References: [DeepSeek Search and Fetch](https://api-docs.deepseek.com/guides/search/),
the [Search schema](https://api-docs.deepseek.com/api/search/), the
[Fetch schema](https://api-docs.deepseek.com/api/fetch/), and
[upstream comparison 6cf0d90b](https://github.com/QuantumNous/new-api/commit/6cf0d90b5ff27f901697dddd61e0c28aba8efefc).

### DeepSeek reasoning controls

Chat, Responses and Messages adapters require an exact
`ExternalModelContext.deepseek_parameters_contract` revision to enable DeepSeek
controls. A model name or suffix is not authority to change parameters. This
binding is independent of the native Search/Fetch protocol. Client and supplier
protocols must match so that native reasoning history remains representable.

| Client effort | DeepSeek effort |
| --- | --- |
| `none` | `none`, with thinking disabled |
| `minimal`, `low` | `low` |
| `medium`, `high`, `xhigh` | `high` |
| `max`, `ultra` | `max` |

Chat uses `reasoning_effort`, Responses uses `reasoning.effort`, and Messages
uses `output_config.effort` with the supported `thinking.type` control. Only
`enabled` and `disabled` thinking are accepted; conflicting controls, token
budgets and adaptive thinking are rejected. Responses plaintext reasoning
content and its stream events are retained, with an empty summary, instead of
being changed into a different reasoning representation. Chat cache-hit aliases
must agree with any token-detail counters.

The bounded adapter rejects hosted search, prompt-cache lifetime controls,
`parallel_tool_calls: false`, strict function schemas, developer messages and
`top_p`. It also rejects temperature while thinking is enabled, because the
selected contract cannot preserve that sampling request. Unbound routes retain
their existing ordinary effort semantics. References are DeepSeek's
[thinking guide](https://api-docs.deepseek.com/guides/thinking_mode/),
[Responses guide](https://api-docs.deepseek.com/guides/responses_api/) and
[Anthropic-compatible guide](https://api-docs.deepseek.com/guides/anthropic_api/).

## Ali multimodal embeddings

The `ali_multimodal_embedding` protocol has a separate native execution path for
`POST /v1/embeddings` and
`POST /api/v1/services/embeddings/multimodal-embedding/multimodal-embedding`.
The first accepts a string, an array of strings, or typed text/image/video
inputs and returns the OpenAI float-vector envelope. The second retains the
supported native `input.contents` and `parameters` shape. Both dispatch the
native Ali request, validate vector count, dimensions and finite elements, and
use the same reservation and settlement machinery as other model requests.

`EmbeddingTariff` revision 1 has independent exact USD rates per million text
tokens and image/video tokens. They are required even when one rate is explicitly
zero. The provider code fixes the meaning of the native receipt:

| Receipt contract | Native counters | Canonical input |
| --- | --- | --- |
| `ali_separate_input_v1` | `input_tokens` is text; `image_tokens` is image/video | Sum of the two measured counters |
| `ali_inclusive_input_v1` | `input_tokens` is total; `input_tokens_details.text_tokens` and `.image_tokens` are its partitions | Measured total, equal to the two partitions |

All three canonical counters must be known for settlement, including explicit
zeroes. Missing or contradictory partitions do not establish a free dimension.
Ledger input is the measured total; output, cache-read, cache-write and reasoning
usage are zero. Ordinary token, search, image, tiered and upstream-cost prices
are mutually exclusive with the embedding tariff.

This complete `PriceRule` uses **illustrative rates**, not official defaults or
an instruction to enable any particular supplier:

```json
{
  "id": 2,
  "model": "example-multimodal-embedding",
  "group": "default",
  "speed": "standard",
  "billing_mode": "tokens",
  "rates": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0, "request": 0},
  "group_multiplier_ppm": 1000000,
  "speed_multiplier_ppm": 1000000,
  "embedding": {
    "revision": 1,
    "provider": "ali_separate_input_v1",
    "currency": "USD",
    "text_usd_per_million": {"numerator": 1, "denominator": 10},
    "image_usd_per_million": {"numerator": 129, "denominator": 500}
  }
}
```

With those example rates, 10 text tokens plus 20 image tokens cost USD 0.00000616,
or 3.08 credits before discounts. The final charge is 4 credits at unit
multipliers. Core combines both exact amounts, applies the frozen group/speed
multipliers and verified funding discount once, and takes one final ceiling.
Reservation uses the reviewed shared input-token ceiling times the larger of
the two rates, with the same frozen factors. It does not reserve two independent
token ceilings or infer the cheaper partition from the submitted modality.

An exact `ExternalModelContext` must select this protocol and the same provider,
set a supplier-backed `max_input_tokens`, set `max_output_tokens` to zero, and
include an `embedding` contract. The following **synthetic contract fragment**
shows its fields; its limits and dimensions must be replaced with the reviewed
guarantees of the actual endpoint and mapped model:

```json
{
  "embedding": {
    "provider": "ali_separate_input_v1",
    "contract_revision": "reviewed-embedding-supplier-revision",
    "max_contents": 4,
    "max_images": 1,
    "max_videos": 0,
    "max_text_bytes": 4096,
    "max_inline_image_bytes": 1048576,
    "default_dimensions": 1024,
    "dimensions": [1024],
    "supports_dimension_parameter": true,
    "supports_fusion": false,
    "allow_remote_media": false
  }
}
```

Core enforces content, image and video counts, aggregate text/instruction bytes,
decoded inline-image bytes, permitted vector dimensions, fusion and remote-media
permission before reserving or sending. `supports_dimension_parameter` is
mandatory: true sends the selected dimension, including the reviewed default
when the client omits it; false requires exactly one fixed dimension and omits
the native parameter. Restricting a configurable model to one permitted dimension
does not make it a fixed-dimension model. The adapter accepts bounded inline PNG,
JPEG, WebP, BMP and TIFF images; videos use remote URLs. It does not implement
`multi_images`, sparse vectors or arbitrary native options. A contract allowing
remote media must cover the supplier's download validation and maximum billable
input; Core does not fetch those URLs or estimate their tokens locally.

The optional client `max_input_tokens` is a local budget check. Ali's native
operation has no matching enforced token-cap field, so the client budget cannot
reduce the supplier bound or reservation. A value below that bound is rejected
before dispatch; the local field is removed from the provider request. Input
byte/count limits alone do not prove a supplier token ceiling.

References: [Ali multimodal embedding API](https://www.alibabacloud.com/help/en/model-studio/multimodal-embedding-api-reference)
and [upstream comparison afe73b08](https://github.com/QuantumNous/new-api/commit/afe73b08ff6432aca4749a024391c7f886a5bd2d).

## AWS Mantle Messages

Mantle retains the `anthropic_messages` protocol and uses an exact
`ExternalModelContext.mantle_sigv4_region` binding. Configure the reviewed region
alongside the usual exact upstream/model/endpoint and token ceilings. The only
accepted host is `bedrock-mantle.{region}.api.aws`, over HTTPS on the default
port. Root, `/anthropic`, `/anthropic/v1` and the complete messages path are
accepted bases; the adapter normalizes them to `/anthropic/v1/messages`. A
different host, region, operation, query string or fragment is rejected.

The referenced private credential file contains a strict JSON object with
`access_key_id`, `secret_access_key`, and optional `session_token`. It is a
credential revision, not routing or public-catalog data. Pretty-printed JSON is
supported through the dedicated credential loader and existing file-origin
checks. Every use of a credential reference must select the same authentication
scheme; a Mantle JSON reference cannot also be used as a bearer/API key. Keys,
tokens and signatures must not be copied into public configuration examples,
logs or price projections.

Core uses the official AWS `aws-sigv4` and `aws-credential-types` libraries. It
signs the final serialized body bytes for service `bedrock-mantle`, the selected
region and the canonical operation, including the payload hash and temporary
session token where present. Messages uses `anthropic-version: 2023-06-01`;
JSON and SSE responses follow the existing Anthropic receipt and checkpoint
path. There is no fallback that sends the JSON credential as an API key, no
automatic STS credential refresh, and no implicit enablement of hosted search
or other AWS operations. Rotate temporary credentials through the installation's
existing secret-revision and configuration process before they expire.

References: [AWS Messages API](https://docs.aws.amazon.com/bedrock/latest/userguide/inference-messages-api.html),
[AWS Mantle signing service](https://docs.aws.amazon.com/bedrock/latest/userguide/count-tokens.html),
and [upstream comparison d0322047](https://github.com/QuantumNous/new-api/commit/d032204733db31208f10bac118b6e683aab3bf29).
These are Rust implementations of the reviewed contracts. The repository's
[AGPL attribution and upstream provenance](../../../../docs/fork.md) continue
to apply; use of the official AWS libraries does not change that provenance.

### Native-operation recovery and checks

Native tool and embedding usage, frozen prices and terminal outcome evidence use
existing JSON persistence. No DDL or startup migration is required for these
fields. A completed receipt is checkpointed before terminal settlement; recovery
uses the original price, bound, provider and outcome. Missing historical terminal
outcomes retain their original completed meaning. A failed zero-cost outcome is
accepted only for the verified DeepSeek tool contract. Incomplete evidence keeps
the hold for reconciliation. Duplicate IDs, dropped acknowledgements and process
restarts never authorize a second supplier invocation.

Enable new contracts only when all settlement/recovery readers understand their
fields. Older binaries with strict unknown-field rejection cannot safely read
new persisted evidence. A rollback must retain compatible readers; removing
fields or replacing the frozen price is not a recovery procedure.

Focused checks from `services/core`, using the pinned toolchain, are:

```sh
cargo test --locked --lib relay::deepseek_tests
cargo test --locked --lib relay::mantle::tests
cargo test --locked --lib model_http::native_operations
cargo test --locked --lib routing::embedding
# DATABASE_URL must already point to a disposable PostgreSQL server.
cargo test --locked --test native_operations_billing
```

The native integration sources exercise real Core admission and isolated
PostgreSQL ledger state, including insufficient funds before dispatch, complete
and unknown receipts, zero-cost failures, duplicate requests, failed settlement
acknowledgements and restart recovery. The Mantle unit sources include a fixed
public-credential signing vector, endpoint rejection, body/time sensitivity,
session-token signing and secret redaction. Local work has checked formatting,
resolved the locked dependency graph and validated Cargo metadata; these are
not executions of the Rust or PostgreSQL tests. Neither those source checks nor
synthetic suppliers certify a live DeepSeek, Ali or AWS deployment. Use the
recorded CI results for the exact revision as the acceptance evidence.
