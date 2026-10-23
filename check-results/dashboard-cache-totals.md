# Dashboard cache totals verification — 2026-10-22

## Source and contract

- Root owns native Rust `usage.rs` aggregation. This patch owns the dashboard decoder, aggregation, stat cards, eight affected Web files and two descriptions in all seven locales.
- The native response fields are `cache_read_tokens`, `cache_creation_tokens` and optional `cache_creation_tokens_12h`. Read/write cards sum the first two independently. A lifetime subset never adds to the write total or the token total.
- Unknown legacy counters propagate as unavailable; measured zero stays zero. Known image-only buckets do not hide independently measured text totals and do not acquire invented cache measurements. Invalid/negative/fractional/unsafe counters, inconsistent subsets and sum overflow are rejected or unavailable.
- The existing card layout uses four columns on large screens, three on medium and two on narrow screens; labels can wrap and the existing loading/error states remain.

## Actually executed

- 12/12 aggregate tests, 6/6 decoder tests and 7/7 happy-dom component tests passed, all exit 0, after final formatting. The component tests render the real `LogStatCards`, click-free data loading through its actual API adapter and the card DOM. The adapter is mocked; this is not a live server acceptance run.
- Selected-file oxfmt 0.57.0 formatting succeeded on all eight files. The existing upstream license headers were protected using the same pattern as the repository formatter, without a whole-workspace format.
- Translation helper added two descriptions to seven locales and `node scripts/sync-i18n.mjs` completed; every locale reports zero missing and zero extras. The two values per language were inspected. Existing unrelated untranslated keys remain in the sync report.
- `git diff --check -- apps/web/src/features/dashboard apps/web/src/i18n/locales check-results` exited 0.

Reproduction from `apps/web` with the shared temporary dependency symlink:

```sh
node /tmp/lmm-cache-web-check/check-dashboard-cache.mjs
```

That script runs esbuild for each of `lib/stats.test.ts`, `lib/usage-bucket.test.ts`, and `components/models/log-stat-cards.test.tsx` with the repository `tsconfig.app.json`, bundle=true, platform=node, format=esm, packages=external and jsx=automatic, then invokes:

```sh
node --import ./scripts/test-preload.mjs /tmp/lmm-cache-web-check/dashboard-stats.test.js
node --import ./scripts/test-preload.mjs /tmp/lmm-cache-web-check/dashboard-usage-bucket.test.js
node --import ./scripts/test-preload.mjs /tmp/lmm-cache-web-check/dashboard-log-stat-cards.test.js
```

The harness uses test-only `import.meta.env={DEV:false}` and bundles the installed dayjs package to resolve its extensionless plugin imports under Node ESM. Production configuration was not changed. The component now imports its two concrete helper modules directly, avoiding unrelated chart/text barrel side effects.

Reused environment: Node 24.19.0, esbuild 0.27.7, React/ReactDOM 19.2.7, zod 4.4.3, i18next 26.3.4, react-i18next 17.0.8. For the real component DOM test, temporary dependencies were extended only with bun.lock versions `@base-ui/react@1.8.0`, `class-variance-authority@0.7.1`, `lucide-react@1.25.0`, `happy-dom@20.11.1`, `dayjs@1.11.21` and their dependencies. No repository package or lockfile changed.

## Failed attempts retained and unexecuted checks

- The first component invocation failed because the minimal environment lacked dayjs. The next failed on its extensionless plugin import. After bundling dayjs, the broad dashboard barrel pulled dompurify; replacing that import with the two actual helper imports removed that unrelated dependency. Final tests above passed on the corrected source/harness. None of these failures were assertion failures or replaced with successful mock components.
- Full Web typecheck/build, actual browser layout/light/dark/mobile checks and live PostgreSQL-to-browser requests have not run. Backend tests are owned and reported separately by root.
- This patch does not backfill cache counters into immutable historical administrator data; those cards show unavailable when that source lacks measurements.
- The temporary translation patch was deleted. The shared `apps/web/node_modules` symlink is deliberately retained for the other agents' remaining checks; root owns final cleanup.

## Source hashes

- `apps/web/src/features/dashboard/lib/usage-bucket.ts`: `7c2773813baa5e2afc02eaf230cf87b8da4575847fe1f7c41c7be8faf2cd5282`
- `apps/web/src/features/dashboard/lib/stats.ts`: `9662dc4fd3685f79293ca25d9728f188ba538fe91ee2402fa2f800e41621523d`
- `apps/web/src/features/dashboard/lib/usage-bucket.test.ts`: `ba1d581e4c5287d4e10a12cd279f2bf9284b8a3290751c54473a7c22444a434c`
- `apps/web/src/features/dashboard/lib/stats.test.ts`: `1bdf7d0aa08382cecc417524a458ff0e8288bbe41ebad9788d875cebd33cefb8`
- `apps/web/src/features/dashboard/hooks/use-dashboard-config.tsx`: `ed493ded3514212a4805aa336ce333a3ebd720bd619aeaa2334f600247d09479`
- `apps/web/src/features/dashboard/components/models/log-stat-cards.tsx`: `6f42c3b8b3cd5f5b36169f088df1733239360686c2e08d490989dc5b0064ad91`
- `apps/web/src/features/dashboard/components/models/log-stat-cards.test.tsx`: `3ca4c3bac6c0ef7943a818e37d4cf1c0fbd5321594bfc373fff0cde5dfcffc91`
- `apps/web/src/features/dashboard/dashboard-editorial.css`: `e7b901d745b0c3591afacdaff0508e8dd82ecd8edc2a0f55573eb9a429e494f5`
