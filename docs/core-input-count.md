# Responses input reservation profile

The host model file's `external_models[].reservation` selects a strict tagged
union. Omission and `{ mode = "context_max" }` retain the reviewed model input
maximum and make no counter request. Unknown modes/fields and zero/missing exact
timeouts are configuration errors.

The optional `openai_responses_exact_v1` profile requires an explicit host
contract for the official `https://api.openai.com/v1` base, the
`openai_responses` protocol and a fixed dated model snapshot. A dated name only
rejects aliases; it does **not** establish that the provider accepts the model
or that the model's accounting contract is correct.

An illustrative fragment, not a deployment or supplier acceptance result:

```toml
[[external_models]]
upstream_id = 1
upstream_model = "gpt-4.1-2025-04-14"
protocol = "openai_responses"
endpoint = "https://api.openai.com/v1"
max_input_tokens = 1048576
max_output_tokens = 32768
contract_revision = "replace-with-reviewed-contract-revision"

[external_models.reservation]
mode = "openai_responses_exact_v1"
count_timeout_ms = 2000
```

The limits above illustrate syntax; operators must review real model limits and
usage/cache semantics before enabling any contract. This change does not enable
exact counting in a production configuration. Real supplier acceptance is
**NOT_RUN**.

The initial exact profile supports only non-streaming, stateless pure text
generation. The final encoded generation body must contain `model`, `input`,
`store:false`, `stream:false` and a positive `max_output_tokens`; optional
`instructions` must be a string. Input accepts plain strings or complete text
messages with `system`, `developer`, `user` and `assistant` roles. Text blocks
use `input_text`, or `output_text` for an assistant. Every message/instruction
is retained in the count projection.

All other fields are rejected, including tools, stored history/item references,
conversation IDs, media, schemas, cache controls and currently unreviewed
sampling/reasoning fields. Rejection does not delete fields or switch to another
route/profile. Unsupported request options must be rejected before an encoder
can discard them; the route integration owns this admission boundary.

After routing and encoding have frozen the final body, Core sends its complete
`model/input/instructions` projection to the fixed
`https://api.openai.com/v1/responses/input_tokens` endpoint, using the same
credential and existing ProviderTransport. DNS/TLS, pinned origin/peer checks,
no redirects and no proxy apply to the counter as to generation. Gate total
deadline/cancellation covers the exchange; the exact timeout additionally
covers DNS, headers and bounded body reads. Existing connect/header/read/body
limits still apply. The same immutable route supplies the profile, endpoint and
credential. Its availability guard is checked before and after connection
preparation, so route revocation is honored before sending the count request.

Only a `response.input_tokens` object with an unsigned integer not exceeding the
original model input maximum is accepted. No estimate, truncation, retry route
or context-max fallback is introduced. Failure occurs before ambiguous reserve
handling and cannot emit a usage/start permission.

The observation contains the count, requested output cap, versioned projection
identifier and SHA-256 digests of the frozen generation bytes and exact counter
request/response bytes. It contains no raw body, instruction, API key or
Authorization header. The count is admission evidence, not usage or a settlement
receipt. The generator must send the same retained bytes; no post-count mutation
is permitted. The gateway validates the selected route and encoded body again
before preparing the financial contract.

## Durable admission and recovery

Before dispatch, Core stores `reservation-contract:v1` and its
`reservation-admission:v1` marker in the same SERIALIZABLE transaction as the
request, budget checks and wallet hold. The contract fixes the route, credential
reference/revision, wire digest, input/output bounds, cache meaning and charged
price. Neither event contains an accepted `usage` field. A count cannot settle a
request, and an audit-only provider observation cannot replace terminal usage.

The marker and contract must agree. A missing, malformed or mismatched member
blocks dispatch or settlement; recovery preserves the original hold. Existing
unknown requests without this evidence are not backfilled, released or settled
at zero. Old liabilities and separately awarded compensation remain distinct.
Recovery uses the stored terms instead of the current catalog or provider policy.
A lost reservation COMMIT acknowledgement is resolved under the original request
ID; a lost dispatch acknowledgement cannot authorize a second send.

Native System One retains its reviewed full input ceiling and input-only price.
Its separate `systemone_native_v1` admission mode binds the exact bounded JSON or
multipart bytes and marks reported output as unpriced metadata; it does not
claim a generation output limit or use the Responses counter. Generic recovery
still requires its valid original paired evidence.

There is no schema migration in this change. That does not establish binary
rollback compatibility: a recovery worker must understand the stored contract
version and semantics before it can handle newly admitted requests. Deployment
and cutover remain subject to independent business acceptance.

The default `Billing::reserve_prepared` implementation accepts only the existing
context-max path with no count observation. An exact-count route or supplied
count observation requires an explicit prepared-contract adapter; it cannot
silently downgrade to the legacy `reserve` hook.

Targeted local parser/adapter checks:

```sh
mise exec -- cargo test --manifest-path services/core/Cargo.toml --lib input_count
```

Local counter fixtures verify code behavior only. They do not substitute for
official supplier acceptance, nor do they prove production configuration,
deployment or live business behavior.
