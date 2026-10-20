# Native TypeSafe Jev relay

Rust Core owns the synchronous System One relay, authentication, routing and
charging. Structured decisions do not enter a Chat Completions adapter or a
JavaScript plugin host. The historical Go relay is not the WIP implementation.

## Models and configuration

The Web TypeSafe choices are `jev-1.14.0`, `jev-1.13.0`, `jev-latest` and
`jev-preview`. The [official model page](https://docs.typesafe.ai/models), checked
2026-10-21, still maps `jev-latest` to 1.13.0 and `jev-preview` to 1.14.0.
Versioned IDs remain valid even when the provider list returns only aliases.

Core uses explicit model configuration: upstream `protocol = "system_one"`, a
private credential file, enabled group/route/target, mapped upstream model and
reviewed frozen prices. The provider base is `https://api.typesafe.ai`; a base
ending in `/v1` is also accepted. Path prefixes are preserved when appending
`systemone`. Customers cannot supply an upstream URL, provider key, payer or price.

The Web compatibility identifier remains channel type 62; historical OpenHuman
61 is never renumbered. These UI identifiers do not replace Core configuration.
Native catalog API name `system-one`, legacy Web endpoint name `systemone` and
routing value `system_one` are separate contracts requiring explicit mapping.

## Call the API

Both `POST /v1/systemone` and `POST /typesafe/v1/systemone` accept a Core API key
in `Authorization: Bearer`. Current model, group and peer restrictions apply.
Session credentials and existing Pi/model OAuth grants are rejected before a
hold or provider call. The endpoint is synchronous and does not support streaming.

```sh
curl https://YOUR_GATEWAY/v1/systemone \
  -H 'Authorization: Bearer YOUR_LMM_API_KEY' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "jev-1.14.0",
    "state": {"message": "The customer asks to reset a forgotten password."},
    "questions": {
      "account_support": {
        "type": "noul",
        "instructions": "Is this an account support request?"
      }
    }
  }'
```

The native body preserves structured `state`, named questions and optional
`assets`. Types are `noul`, `choice`, `score` and `box`. Choice uses an object of
criteria; score uses an ordered criteria array. Box has no criteria and needs
image input. Answer checks include matching types, probabilities, score legend
and nonzero Box geometry. JSON numbers retain precision and ordinary user keys;
duplicate keys and nonfinite values are rejected.

For SDK decision methods, set the base URL to `https://YOUR_GATEWAY` or
`https://YOUR_GATEWAY/typesafe` and supply the model explicitly. **The gateway's
`/v1/models` keeps its OpenAI-shaped response. The TypeSafe SDK `models.list()`
contract is not implemented.** Successful decisions do not prove list compatibility.

## Images and long context

Images require `TypeSafe-Danger-Zone-Images: true` on each request. Box requires
Jev 1.14 (the explicit version or corresponding preview) and actual image input.
Assets accept inline `data_base64` with PNG/JPEG format or a multipart part
reference. Multipart begins with the JSON `request` part, `application/json`;
unique image parts must match asset references one to one. References occur in
string values, including nested state/instructions, never object keys.

Wire and decompressed bodies are each capped at 32 MiB. Each base64-decoded or
uploaded image file is capped at 16 MiB. Local checks inspect PNG/JPEG headers
and dimensions 280..1568 and reject animated PNG. Full image decoding and the
provider's re-encoded image and expanded-reference limits remain provider
checks. Raw bytes multiplied by reference count do not prove processed size.
See the [official image contract](https://docs.typesafe.ai/images).

Single-layer `gzip`, `x-gzip` and `zstd` are decoded before JSON/multipart parsing;
zstd windows are capped at 8 MiB. Invalid, stacked and oversized encodings are
rejected. This includes SDK compression of the entire multipart body. Parsing
and decompression share a gate allowing at most two System One requests.

Long context also requires `TypeSafe-Danger-Zone-Long-Context: true` and a reviewed
larger host reservation bound. A request header cannot raise the installed bound.
The official `262k` abbreviation is not used to guess an exact reservation.
External model contexts must match protocol, endpoint, model and contract revision,
with `missing_cached_tokens_is_zero = true` for the System One usage contract.

## Charging and failures

The official price checked on 2026-10-21 is $0.042 per million input tokens;
output is free. Images contribute input tokens, not an image-generation charge.
These provider facts do not rewrite saved customer prices or price locks.

Core uses the installed frozen input-token tariff. Output, discounted-cache and
per-request rates must be zero; unsupported dimensions are rejected. The WIP
implementation does not restore historical expression pricing.

The input reservation ceiling must be at least 65,536 tokens. Core reserves
against the configured ceiling, validates the complete response and integer input
usage within its reviewed bound, persists the terminal observation, then settles
before returning answers. Nonzero output usage adds no output charge. Explicit
zero input usage is distinct from missing or malformed usage.

Invalid usage, incomplete responses and uncertain provider work keep the original
hold for reconciliation. They are not treated as free requests or guessed success.
Send a stable `Idempotency-Key` for an operation. Reusing it returns a conflict
instead of resending upstream; omitting it creates a new operation per request.
A disconnected caller does not stop the Core worker from recording the outcome.
Draining tracks these workers. Responses use `Cache-Control: no-store`.

## Evidence and provenance

References are the [System One API](https://docs.typesafe.ai/api/system-one),
OpenAPI and official SDKs. The 2026-10-20 OpenAPI snapshot SHA-256 is
`ee599512cbea3a198b34af3f60addcd72b381d3521196f93c70ae83185c057c3`.
Reviewed MIT SDKs: JavaScript 0.7.0 revision
`b844cd1aca12855a5d24a2df52a1401c86e7d37a`; Python 0.7.5 revision
`f36e68549ceca3191e5791e5f99ea1baa1a258b5`.

Historical Go behavior came from Apache-2.0 TypeSafe 1.0.0 at
`QuantumNous/new-api-plugins` revision
`97a16e9a8d98b73ffb76e1c5a00b4d0f855f4d0a`; attribution is retained.
No executable plugin source or task host is imported.

Local synthetic-provider/PG tests prove only their tested routing and charging.
Independent SDK acceptance, live provider acceptance and deployment acceptance
are separate outcomes. Provider access/resale permissions are separate from
software support; see the [TypeSafe MCA](https://typesafe.ai/legal/mca).
