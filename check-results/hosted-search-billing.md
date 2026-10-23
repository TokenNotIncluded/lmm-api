# Hosted search billing: developer checks

This record describes source development, not independent acceptance. The
candidate extends the 12-hour cache draft based on `451f50b0`; its four missing
test initializer fields were separately fixed in `7ef2eb89`. No deployment or
production configuration change was performed.

## Executed

- Two search-price schema/SSR tests passed: supplier/unit validation, exact
  fractions, availability, group filtering and Chinese/English rendering.
- Three usage-counter tests passed: missing versus zero, token availability,
  wrong supplier, malformed values, contradictory counters and separate xAI
  post/user measurements.
- Fifteen existing pricing-catalog contract tests passed with the new optional
  search projection, including the existing 12-hour cache tariff regression.
- Changed Rust files were parsed/formatted with the official Rust 1.99.0
  rustfmt binary, `--edition 2024 --config skip_children=true`. This is not a
  Rust type check or test execution.
- Search translations used the repository helper for all seven locales;
  `node scripts/sync-i18n.mjs` succeeded from `apps/web`. An initial sync from
  the repository root failed on the wrong working directory and did not write
  application files; the corrected invocation is the recorded success.

Targeted Web commands, from `apps/web`, used Node 24.19.0 and the minimal existing
test dependencies in `/tmp/lmm-cache-web-check`. esbuild bundled the three
test entry points with `platform: node`, `format: esm`, `packages: external`,
`jsx: automatic`, the repository `tsconfig.app.json`, and a test-only
`import.meta.env={DEV:false}` definition for the existing i18n initialization.

```sh
node --import ./scripts/test-preload.mjs /tmp/lmm-cache-web-check/search-tests/pricing/components/search-prices.test.js
node --import ./scripts/test-preload.mjs /tmp/lmm-cache-web-check/search-tests/usage-logs/lib/search-usage.test.js
node --import ./scripts/test-preload.mjs /tmp/lmm-cache-web-check/search-tests/pricing/catalog-contract.test.js
```

## Source regression coverage awaiting execution

The Rust test sources cover exact combined rounding/hold ceilings, finite
supplier contracts, scalar/tiered prices, Gemini's alternative units, xAI's
independent dimensions, missing/zero/malformed and unsupported measurements,
cumulative stream evidence, frozen JSON restoration and native output metadata.
`tests/search_billing.rs` additionally uses actual Core HTTP, isolated PostgreSQL
and ledger fixtures for native JSON/SSE, pending unknown receipts, concurrent
duplicates, discarded client acknowledgements, restart recovery and insufficient
combined reservation before supplier bytes.

The local environment has no complete Cargo/Rust build toolchain or disposable
PostgreSQL. These Rust tests have **not been executed locally**. Full Web
typecheck/build, browser layout/interaction checks and live-supplier acceptance
have **not been executed**. Automatic CI results must be read separately against
their exact commit; failures are not superseded by formatter or SSR success.

CSV export and dashboard cache totals have their own adjacent check records.
