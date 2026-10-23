# Usage CSV export verification — 2026-10-22

Source baseline: `451f50b07880051f4c0de43ba99b7c1e2e1b8089`, with the uncommitted CSV patch listed below. Other agents are changing search billing in the same worktree. These results cover only CSV files, not the complete branch.

## Implemented contract

- `GET /api/log/self/export` uses the native log page's current browser-session, actor and personal-owner authority. `GET /api/log/export` reuses the administrator historical log source and its existing current-role/filter validation. The sources are explicitly distinguished in the response and filenames.
- Both require explicit ordered timestamps spanning at most 31 days. Pagination parameters are rejected; the Web client removes them and sends all applied filters.
- At most 10,000 rows / 32 MiB per file, one active export per authenticated user / four per Rust process. Native database rows are streamed, receipts are validated, and a complete bounded CSV is built before returning successful headers. Historical export reuses the existing bounded snapshot reader. No schema changes.
- Generation and transfer each have a 60-second deadline. Client timeout is 125 seconds. Disconnect or stalled-body timeout drops the full buffer and admission lease; a truncated body yields an error and cannot terminate successfully. A bounded channel holds no more than two copied 64 KiB chunks beyond the full buffer's lifetime.
- UTF-8 BOM, RFC 4180 quoted cells, and formula protection after whitespace/control/BOM trimming. Unknown token/money/search values remain blank; verified zero remains `0`. Native search provider plus all five receipt counters are included. No raw credentials, provider payload, internal fingerprint or command documents are exported.
- Web discards cancellation, old account/session responses, malformed headers, source mismatches and incomplete byte counts. A Blob/download is created only after complete validation. Existing toolbar and seven locales are reused.

## Checks actually executed

1. **8/8 Web transport/helper tests passed, exit 0** on Node v24.19.0. Cases: filters without pagination; complete self request; admin source/filter contract; wrong source/non-CSV/missing metadata/truncation; cancellation; account switch; oversized transfer; actionable 400/401/403/429/504 states without retries. These tests mock the Axios transport and do not claim a live backend acceptance run.
2. **Component dependency graph transpile/bundle succeeded, exit 0** with external packages. This is syntax/import processing, not TypeScript type checking, browser behavior or SSR acceptance.
3. **oxfmt 0.57.0 check passed on the three new TypeScript/TSX files, exit 0.** The existing toolbar file was formatted with its upstream license header preserved.
4. **Translation helper applied 13 keys to all seven locales; i18n sync succeeded.** Report has zero missing and extra keys. Values and interpolation were reviewed. No browser language/layout checks were executed.
5. **Portable rustfmt 1.99.0 formatting/parsing passed, exit 0**, on `csv_export.rs` and `csv_export_tests.rs`, using `LD_LIBRARY_PATH=/tmp/lmm-rust-format/rustc-1.99.0-x86_64-unknown-linux-gnu/rustc/lib /tmp/lmm-rust-format/rustfmt-1.99.0-x86_64-unknown-linux-gnu/rustfmt-preview/bin/rustfmt --edition 2024 --config skip_children=true <files>`.
6. **Tracked CSV/locale diff whitespace check passed, exit 0.** New-file `git diff --no-index --check /dev/null <path>` produced no whitespace diagnostics (exit 1 denotes added content).

### Reproduction commands

From `apps/web`, with a temporary `node_modules` symlink to `/tmp/lmm-cache-web-check/node_modules`:

```sh
node --input-type=module <<'NODE'
import { build } from '/tmp/lmm-cache-web-check/node_modules/esbuild/lib/main.js'
const options = { absWorkingDir: process.cwd(), bundle: true, format: 'esm', packages: 'external', jsx: 'automatic', define: { 'import.meta.env': JSON.stringify({ DEV: false }) }, tsconfig: 'tsconfig.app.json' }
await build({ ...options, entryPoints: ['src/features/usage-logs/lib/csv-export.test.ts'], outfile: '/tmp/lmm-cache-web-check/usage-csv-export.test.js', platform: 'node' })
await build({ ...options, entryPoints: ['src/features/usage-logs/components/usage-csv-export.tsx'], outfile: '/tmp/lmm-cache-web-check/usage-csv-control.mjs', platform: 'browser' })
NODE
node --import ./scripts/test-preload.mjs /tmp/lmm-cache-web-check/usage-csv-export.test.js
node /tmp/lmm-cache-web-check/node_modules/oxfmt/bin/oxfmt --check src/features/usage-logs/lib/csv-export.ts src/features/usage-logs/lib/csv-export.test.ts src/features/usage-logs/components/usage-csv-export.tsx
```

The `import.meta.env.DEV=false` injection is test-only and permits the existing browser i18n module to load in Node; no production environment setting was changed. Reused minimum dependencies include React/ReactDOM 19.2.7, react-i18next 17.0.8, i18next 26.3.4, zod 4.4.3, axios 1.18.1, zustand 5.0.14, sonner 2.0.7, oxfmt 0.57.0; these application/formatter versions match this repository. The harness uses esbuild 0.27.7. No full workspace install was performed.

From repository root, translations were applied using the repository helper and then synced:

```sh
node .agents/skills/i18n-translate/scripts/apply-translations.mjs /tmp/lmm-usage-csv-translations.json
cd apps/web
node scripts/sync-i18n.mjs
```

## Unexecuted checks and limits

- Rust compiler/Cargo were unavailable in this environment. **No Rust compilation or PostgreSQL test ran.** Portable rustfmt 1.99.0 subsequently formatted and parsed both new CSV Rust files successfully (edition 2024, skip_children=true); formatting is not type checking. Eight Rust tests were added: three pure contracts (CSV/unknown/zero/formulas, time/byte caps, search counters), two async admission/cancellation/timeout cases, and three actual SQLx isolated-database integration cases (owner isolation/full rows/filters/key denial, corruption/density, administrator historical source/filter/role checks). Test source is not passing evidence. Intended command when an isolated `DATABASE_URL` and repository toolchain are available: `cargo test --manifest-path services/core/Cargo.toml usage::csv_export::tests`.
- Web full typecheck, full build, SSR, real browser and live authenticated API download checks were not run. The minimal environment does not include the complete application dependency tree.
- Concurrency admission is process-local. It is not a distributed cross-node quota.
- Existing legacy administrator history and native personal receipts remain separate sources, matching the current pages. This does not introduce a site-wide native receipt administration page.
- Independent source review by the parallel tariff reviewer reported no authority/formula/partial-output defect in the reviewed CSV code; review is separate from the unexecuted runtime acceptance checks above.

## CSV source hashes at handoff

- `services/core/src/usage/csv_export.rs`: `0ff5e76921f28bfb68e2a375a76f3985716afba1ffc2513bfefd51ac3c329184`
- `services/core/src/usage/csv_export_tests.rs`: `fb8bc5a587889f6c9d3b9095d6c1b3c63069258883a74cf38593e0ac917c22f1`
- `apps/web/src/features/usage-logs/lib/csv-export.ts`: `479d3a1daafa95db9640d67e8cc32e1425b3992949792f42ac630a7f6db3e941`
- `apps/web/src/features/usage-logs/lib/csv-export.test.ts`: `474940d465178ce3307f3c7424cca375ff43135218fdc8f8217659e986933bd8`
- `apps/web/src/features/usage-logs/components/usage-csv-export.tsx`: `5dfd444fccbf2b6892766ac5a9951fb5b79e1fdd523a04fd01779e84b12da8b9`
- `apps/web/src/features/usage-logs/components/common-logs-filter-bar.tsx`: `8a61ab7acb53a288f2c1f1a00105e96210193e097ef7d2e8cccdc3435bcbd6cd`
