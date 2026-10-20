/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */

export const LMM_ISSUER = 'https://api.lmm.best'
export const LMM_SOURCE = 'https://github.com/TokenNotIncluded/lmm-api'

// Copyable technical artifacts deliberately use English; the surrounding UI is localized.
export const PRICING_PROMPT = `Integrate LMM model pricing into this project. First inspect its framework, HTTP client, authentication and tests; reuse the existing stack and check dependencies before adding anything. Explain your changes in the language of our conversation.

Provider: https://api.lmm.best
1. Public catalog: GET /core/v1/pricing/public. Access follows the site's visibility settings; handle 401/403 explicitly. Browser account catalog: GET /core/v1/pricing/session with the current Session credential. Never send an OAuth bearer or API key as a browser Session. Both return a versioned document directly, not the old success/data/model_ratio envelope; validate schema_version, visibility, routing_version, pricing_version, groups and models at the boundary.
2. Preserve exact model IDs, names, group identities, supported APIs and pricing units. Group-specific prices already include their configured group factors: do not multiply them twice or combine different group rates into one invented base price. Only show protocols actually listed for that model. Do not infer supported routes or prices from a model name.
3. For an already registered OAuth client, GET /api/oauth2/catalog with Authorization: Bearer ACCESS_TOKEN and an approved grant including catalog:read. This separate endpoint returns {schema_version, resource, updated_at, routing_version, pricing_version, groups, models}. Do not send browser cookies, Session credentials or API-key headers with OAuth bearer credentials.
4. Native token prices declare currency, unit and rate_basis; the separate OAuth pricing object calls its basis price_basis. Treat null/unknown prices as unavailable, not zero. Image variants have distinct size/quality and exact integer-ratio prices per image; they are not token prices or a guarantee that an arbitrary image request is billable. Keep all pricing dimensions distinct and show when final cost depends on usage. A catalog or configuration snapshot is not live provider health or permission to invoke a paid model.
5. Fetch through this project's backend when cross-origin browser access is not supported. Use bounded timeouts, cancellation, a short cache and Retry-After-aware bounded retries for 429/temporary failures. Scope personalized caches to the authorized account and grant. Show a recoverable error, never an endless loading/retry screen or fabricated prices.
6. Add HTTP boundary tests for success, empty catalog, null versus zero prices, different group rates, token versus per-image units, 401/403, 404, 429, timeout and invalid JSON. Do not make paid model requests to test the catalog.

Source contracts:
https://github.com/TokenNotIncluded/lmm-api/blob/wip/rust-core-go-extensions/services/core/src/pricing_http.rs
https://github.com/TokenNotIncluded/lmm-api/blob/wip/rust-core-go-extensions/services/core/src/model_http/pricing.rs
https://github.com/TokenNotIncluded/lmm-api/blob/wip/rust-core-go-extensions/services/core/src/model_http/catalog.rs
https://github.com/TokenNotIncluded/lmm-api/blob/wip/rust-core-go-extensions/services/core/src/oauth2_http.rs
Deliver the implementation, configuration example without credentials, and test results.`

export const OAUTH_PROMPT = `Integrate LMM OAuth into this project where the provider contract supports it. First inspect the application type and existing auth libraries; check installed dependencies and reuse the current stack. Explain your work in the language of our conversation.

Trusted issuer: https://api.lmm.best
Discovery: GET /.well-known/oauth-authorization-server
Resource metadata: GET /.well-known/oauth-protected-resource/api/oauth2
Authorize: GET /api/oauth2/authorize
Token: POST /api/oauth2/token
Revoke: POST /api/oauth2/revoke
Resource: https://api.lmm.best/api/oauth2

Confirm prerequisites before writing an unusable login flow. Use only a client advertised by discovery. Current Core supports the read-only lmm client and optionally enabled lmm-pi; other applications require server support. Do not borrow these IDs or invent a client secret. There is no public dynamic registration, OpenID Connect discovery, userinfo endpoint or ID token. If the goal is third-party website identity/SSO, explain this missing provider capability instead of pretending a resource access token identifies a website user.

For a registered native client:
- Use Authorization Code with PKCE S256 and a cryptographically random state/verifier. Bind the loopback callback listener before opening the system browser. The registered native callback is http://127.0.0.1:<nonzero-port>/oauth/lmm/callback with the actual ephemeral port; localhost, missing ports, query and fragment are rejected. Verify the exact registered contract for this client.
- Send response_type=code, client_id, redirect_uri, resource, the approved scope profile, state, code_challenge and code_challenge_method=S256. Validate both returned state and iss against the original state and trusted issuer.
- Exchange a single-use code with form-encoded grant_type=authorization_code, code, client_id, redirect_uri, resource and code_verifier. Treat tokens as opaque. Do not log authorization URLs, codes, state or credentials.
- Store credentials using this application's secure native storage, persist the returned scope and expires_in, and serialize refresh across processes. Refresh rotation has no grace period: do not replay an old refresh token after an ambiguous network failure; request reauthorization. Revoke on disconnect and erase local credentials.
- Use granted capabilities only where the approved client profile permits them. Do not substitute dashboard cookies, Session credentials or raw API keys for OAuth tokens. The built-in lmm profile requires exactly catalog:read balance:read. Enabled lmm-pi requires catalog:read balance:read usage:read models:invoke with a finite server-enforced budget; additional scopes do not turn another client into Pi. Claude/Gemini native examples use API keys, not OAuth bearer tokens.
- Test callback cancellation, mismatched state/issuer, expired/replayed code, denied scopes, timeout, refresh concurrency, token revocation and unavailable discovery with mocks. Do not request a user's password or API key in chat.

Source contract:
https://github.com/TokenNotIncluded/lmm-api/blob/wip/rust-core-go-extensions/services/core/src/oauth2_http.rs
https://github.com/TokenNotIncluded/lmm-api/blob/wip/rust-core-go-extensions/services/core/src/identity/grants/protocol.rs
Deliver code only for supported, registered flows; clearly list remaining provider prerequisites and test results.`

export const PRICING_EXAMPLE = `export async function getLmmPricing() {
  const response = await fetch(
    'https://api.lmm.best/core/v1/pricing/public',
    { signal: AbortSignal.timeout(10_000) }
  );
  if (!response.ok) throw new Error('Pricing: HTTP ' + response.status);
  const catalog = await response.json();
  if (catalog.schema_version !== 1 || catalog.visibility !== 'public' ||
      !Array.isArray(catalog.models) || !Array.isArray(catalog.groups)) {
    throw new Error('Invalid pricing response');
  }
  return catalog;
}`
