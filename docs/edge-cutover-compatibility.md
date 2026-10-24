# Public edge compatibility preflight

This is a source-level route inventory for a future Core/Extensions cutover. It
is a review aid, not an Nginx site configuration. The repository does not carry
the current public TLS server block, so a cutover cannot be approved from these
examples alone.

## Listener and ownership

| Process | Listener in the native path | Public paths it may own |
| --- | --- | --- |
| Core (`lmm-core`) | `8080` by default (`127.0.0.1:18080` in the Docker compose mapping) | identity, model, OAuth, pricing, history, remote-control relay, and Core health |
| Extensions (`lmm-extensions`) | `8081` by default (`127.0.0.1:18081` in the Docker compose mapping) | Web compatibility routes, payment callbacks, and service-token module routes |
| Web | Nginx-readable static `dist` | `/static/` immutable assets and the SPA fallback at `/` |

Core and Extensions communicate through a private Unix socket and service
credentials. The edge must never proxy `/internal/*`, expose `/extensions/v1/*`,
or send a host/service token to a browser.

## Core public contract

These paths are assembled by `services/core/src/main.rs` when their component is
configured:

- Health and status: `/health/live`, `/health/ready`, `/health/model-ready`,
  `/api/status`.
- Native identity and administration: `/core/v1/auth/*`,
  `/core/v1/identity`, `/core/v1/keys`, `/core/v1/credentials*`,
  `/core/v1/teams*`, `/core/v1/invites*`, `/core/v1/admin/*`, and
  `/core/v1/authz/*`.
- Legacy browser identity: `/api/user/*`, `/api/token*`, and
  `/api/oauth/{google,github,linuxdo,discord,telegram}*`.
- OAuth: `/.well-known/oauth-authorization-server`, `/api/oauth2/*`, and the
  optional market OAuth routes under `/api/oauth2/market/*`.
- Model and protocol routes: `/v1/models`, `/v1/chat/completions`,
  `/v1/responses`, `/v1/messages`, `/v1beta/models/*`, `/v1/images/*`, and
  `/v1/systemone` (plus its `/typesafe` compatibility path when enabled).
- Core-owned browser surfaces include `/core/v1/pricing/*`, selected wallet,
  bounty, history (`/api/log*`, `/api/data/self`, `/api/finance/export`,
  `/api/mj*`, `/api/task*`) and human-grant routes.
- Remote control is exposed at `/api/remote-control/v1/pi/*`; Core authenticates
  the browser request and relays to the private Extensions origin.

Core's fallback is a 503 migration/readiness error. API errors must never fall
through to the SPA `index.html`.

When Core `edge_access` is enabled, the loopback proxy request must carry
exactly one configured `X-LMM-Edge-Proof` and `X-LMM-Edge-Client-IP` header.
Core strips these before dispatch. The edge must not forward either header to
Extensions or a provider. Missing, duplicate, or non-loopback proof is a 403;
therefore a direct health probe without the trusted-edge contract is not a
cutover failure by itself.

## Extensions public contract

With `web_compat=true`, the Extensions Web handler owns these exact families:

- `/api/store/*` (including support conversations and customer views),
  `/api/user/auth/store-claim/*`, and `/api/user/payments/orders/*`.
- `/api/tool-market*` and `/mcp/market`.
- `/api/assistant/*` and `/api/assistant/services/*`.
- `/api/release-notes/*`, `/api/scripts*`, and `/scripts/*`.
- Builtin wallet/bounty compatibility paths: `/api/wallet*`,
  `/api/wallet-transfer*`, and `/api/open-source-bounties*`.
- Legal pages are only the exact GET/HEAD paths
  `/api/user-agreement`, `/api/privacy-policy`, and `/api/refund-policy`.
- Tool-market's operator callback is the exact
  `/api/tool-market/runtime/oauth/callback` path. Preserve its query keys,
  state cookie, and no-cache behavior; it is not a generic redirect endpoint.
- Provider callbacks are a separate callback-only mount:
  `POST /callbacks/{channel}`. It must have its own rate/body limits, no
  query/body logging, and no host credential.

The service-token API under `/extensions/v1/*` is an internal contract, even
though the process listens on TCP. It is not a public edge route. Extensions
health (`/health/live`, `/health/ready`) only proves the Go host is alive; it is
not proof that every selected module or database is ready.

## Streaming and upgrade behavior

Core model responses and remote-control events use HTTP streaming/SSE. The
remote-control SSE response is `text/event-stream` with `X-Accel-Buffering: no`.
The edge candidate must disable proxy buffering for these locations, allow a
stream's intended lifetime, and preserve request/response cancellation while
keeping bounded connect, header, body, and send-backpressure timeouts. Do not
add a total response timeout that cuts a valid stream during normal operation.

The current Core status explicitly advertises
`backend_capabilities.responses_websocket=false`, and no public WebSocket route
is assembled in this revision. A client probing a Responses WebSocket is not
proof that an Upgrade proxy is required. Preserve an Upgrade block only for a
separately verified legacy route.

The remote-control relay is still a source/WIP contract and has no independent
public OAuth plus edge acceptance in this revision. Keep that route out of a
cutover sign-off until the browser OAuth flow, public adapter, and end-to-end
edge SSE test are all independently recorded.

## Safe candidate preflight

Before any host change, obtain the actual backed-up TLS server block and record
its hash. Review the candidate against that frozen input:

1. Enumerate every existing `location` and upstream. Add Core, Extensions,
   callback, and static locations explicitly; do not replace the server block
   with `deployment/native/templates/web-location.conf`.
2. Confirm exact-prefix ownership for the route families above. In particular,
   split Core `/api/user/*`, `/api/oauth*`, `/api/status`, history and remote
   control from Extensions `/api/store/*`, `/api/tool-market*`, assistant,
   scripts, wallet/bounty and payment paths. A broad `/api/` proxy is unsafe.
   Keep the three legal-page locations and the tool-market OAuth callback ahead
   of generic API or SPA handling.
3. Confirm `/internal/*`, `/extensions/v1/*`, private Unix sockets, and service
   credentials are unreachable from the public listener.
4. Validate static handling: `/static/` reads the append-only hashed asset union
   with long immutable caching; `/` serves the selected `index.html` with
   no-cache. Unknown API paths must return the backend error, not the SPA.
5. Run `nginx -t` against the candidate while the current site remains selected.
   In an isolated fixture, exercise health, one representative Core API, one
   representative Extensions API, a signed `/callbacks/{channel}` request, an
   SSE response, and a negative request to `/internal/*`. Record status,
   `Content-Type`, `Cache-Control`, `Retry-After`, `X-Accel-Buffering`, and
   upstream process identity.
6. Independently check Core `/health/ready` and `/health/model-ready` semantics.
   A 200 readiness response is component-scoped and does not prove migration,
   financial reconciliation, browser acceptance, or upstream provider health.
   If `edge_access` is enabled, health/API probes also need the configured
   trusted-edge proof and client-IP headers; a direct 403 is expected without
   them.
7. Keep the existing site serving until migration, drain, backup/restore,
   final-writer fence, financial reconciliation, independent browser journeys,
   and process/artifact identity checks are complete. Selecting `active.conf`
   or reloading Nginx is a final operator action, not part of this preflight.

### Local executable harness

The repository now includes a localhost-only fixture harness:

```sh
python3 -B scripts/test_edge_cutover_harness.py
```

It starts disposable Core and Extensions HTTP stubs plus a temporary Nginx
instance, then checks ten candidate-edge properties: Core/Extensions route
precedence, OAuth query preservation, callback body preservation and host-token
stripping, private-route denial, API-vs-SPA fallback, immutable static assets,
SSE content type and first-chunk timing, and `nginx -t`. It binds every listener
to `127.0.0.1` and removes the temporary fixture after the run. This proves the
candidate contract and catches Nginx precedence regressions; it does not prove
the production host's TLS server block or module configuration.

## Current blockers

- No tracked production Nginx/TLS server block or candidate upstream map exists
  in this repository; the host inventory is required to check route precedence,
  headers, buffering, certificate renewal, and legacy paths.
- The local fixture harness covers Nginx routing, SSE/callback behavior, and
  static/API fallback separation. A real host acceptance run is still required:
  the repository does not contain the production TLS server block, upstream
  inventory, certificate state, or host-specific module selection.
- `/extensions/v1/*` and callback exposure still depend on the host-specific
  web-compat/module selection. A source route list cannot establish that the
  running Go process has the same modules, callback channel, database, or
  service credential.
- The migration and final writer-fence gates remain incomplete. Health, package,
  `nginx -t`, or a successful local request cannot be reported as a cutover.

References: `deployment/native/README.md`,
`deployment/native/templates/web-location.conf`,
`services/core/src/main.rs`, `services/core/src/model_http.rs`,
`services/core/src/remote_control.rs`, `services/core/src/http.rs`, and
`services/extensions/internal/webcompat/router.go`.
