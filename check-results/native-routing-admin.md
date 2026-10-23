# Native routing administration verification — 2026-10-22

## Implemented contract

The native administrator entry points are `GET /api/core/routing-admin/catalog`, `GET /api/core/routing-admin/prices/export`, `POST /api/core/routing-admin/prices/preview`, `POST /api/core/routing-admin/priorities/preview`, `POST /api/core/routing-admin/priorities/undo-preview`, and `POST /api/core/routing-admin/commit`.

Prices currently import the explicit `core-native-price-source.v1` USD format exported by the native service. Per-model keep/multiply/divide affects only selected current model/group/speed rules. The exact, revisioned `USDExchangeRate` option is captured for multiply/divide; absent, invalid, changed or unrepresentable values reject publication. Native token, tiered cache, search and multimodal embedding amounts are covered without modifying their billing contract. Flat micro-USD amounts must remain exactly representable. Historical snapshots remain immutable. This iteration does not accept a legacy New API ratio response or fetch a remote site's ratio endpoint; that compatibility remains under investigation.

The channel dialog reads the real native routing release. It supports table/card placement, same/above/below priority previews, keyboard controls and a new-version undo. Smaller numeric priorities are preferred, matching the Rust router. Weights and all non-priority target fields are preserved.

Server-created previews bind the exact expected release and whole snapshot digest, current native administrator Session/MFA/policy authority, source/FX evidence, the complete financial preview and its confirmation digest. Publication rechecks authority and CAS under one transaction; retries use one operation identifier. Notification creation is in that same transaction. Startup and subsequent admission read durable active state; a missing durable release after previous publications fails closed. The read-active wrapper supports an existing read-only transaction. Root owns the startup/admission and dynamic pricing/OAuth catalog integration.

Component 27 introduces private preview/operation storage and five narrow definer wrappers with `search_path=pg_catalog`. It also qualifies `public.` relations in five existing routing functions while preserving their algorithms and invoker semantics, and expands the durable protocol CHECK to the seven actual Rust variants. The runtime receives schema USAGE and exactly five wrapper EXECUTE grants from the explicit installer template. It receives no direct table/column/sequence access, helper EXECUTE or grant options. The startup checker rejects excess authority even when models are disabled. Test-role provisioning reuses the same marked installer block.

## Independent authority review and correction

Independent review found that the original first-publication SQL branch trusted runtime-supplied `plan.before` as the initial baseline. Although HTTP built that document from memory, the narrow definer boundary was incomplete. That branch has been removed. Both wrappers now reject absent durable active state with P2703. The explicit offline `configure-routing-baseline` command validates the schema and actual table owner, then atomically registers references/stages prices/publishes the first release. Identical repetition is idempotent; an existing different release is rejected. No runtime grant was broadened. A real restricted-role regression attempts a forged origin/reference baseline through both wrappers and verifies that no routing state is created.

## Actually executed

- Native Web tests: 9 helper/contract + 4 API + 3 actual publication-review SSR tests passed, 16 total, exit 0. Cases cover exact fractions/large values/measured zero/group and speed multipliers, all priority placements, authority changes/abort, unchanged idempotency input, stale output identity and expired confirmation.
- `price-sync.tsx` and `priority-sort.tsx` esbuild syntax bundles passed after final table-row markup changes. These are compile/syntax checks, not browser interaction checks.
- Selected Web files were formatted with oxfmt 0.57.0, preserving the existing upstream license headers. Rustfmt 1.99.0 parsed/formatted the Rust source and the new ACL checker with edition 2024 and skip_children=true.
- The translation skill helper applied 66 keys to all seven locales, then the repository i18n sync completed with zero missing/extra keys. Existing unrelated untranslated keys were not reclassified as translated. Root subsequently owns additional locale changes.
- `git diff --check` on the shared installer/checker/role fixture/boundary script changes exited 0.
- `python3 scripts/test-core-boundaries.py CoreBoundaryTests.test_every_core_schema_and_future_core_schema_remains_forbidden` passed after adding routing-admin and the two already-declared Telegram/trust-pricing schemas to the protected list.

Web reproduction from `apps/web` in the temporary minimum dependency environment:

```sh
node /tmp/lmm-cache-web-check/check-routing-admin.mjs
```

The harness bundles `helpers.test.ts`, `api.test.ts` and `publication-review.test.tsx` with esbuild: bundle=true, platform=node, format=esm, packages=external, jsx=automatic and the repository tsconfig. It injects only test-time `import.meta.env={DEV:false}`. Each generated bundle runs with Node `--import ./scripts/test-preload.mjs`. The test components use the real publication-review implementation and existing Button/Table components.

Used dependencies match the repository lock: Node 24.19.0; esbuild 0.27.7; React/ReactDOM 19.2.7; zod 4.4.3; i18next 26.3.4; react-i18next 17.0.8; axios 1.18.1; zustand 5.0.14; @base-ui/react 1.8.0; class-variance-authority 0.7.1; lucide-react 1.25.0. No whole-workspace dependencies were installed or repository lockfile changed. The shared node_modules symlink remains owned by root for its remaining checks.

## Unexecuted checks and remaining gates

- This environment has no usable local Cargo/PostgreSQL toolchain. The eight pure Rust tests, three native HTTP/publication/owner-bootstrap SQLx tests, and two ACL SQLx tests are written but have not run here. Rustfmt parsing is not a Rust compile or SQL validation result.
- The ACL tests use a real restricted database role, revoke each required schema/function grant, reject table/column/helper/PUBLIC/grant-option escalation, and execute read-active inside a read-only transaction. They are intended for CI, not claimed as passed.
- Full Web typecheck/build, main-dialog happy-dom interaction and actual browser layout/accessibility checks have not run here. No large icon dependency was installed to expand the temporary environment.
- The first complete boundary-script run executed 31 tests and reported three failures: the two missing declared schema names were fixed, while two failures concern the existing independently-reviewed SHA pin of `services/extensions/internal/freshinstall/install.go`. That pin was not changed by this work. The focused schema-coverage test above passed afterward; the complete suite is not claimed green.
- **Existing-install 26→27 upgrade is not accepted yet.** `routing_admin_predecessor26_catalog_evidence` constructs the exact 26-component predecessor, prints PostgreSQL version, manifest/source digests and actual catalog fingerprint, then intentionally fails with `PIN_PENDING`. The trusted predecessor pin and explicit upgrade acceptance must follow review of actual PostgreSQL 18 evidence. No catalog hash was invented or dynamically accepted, and no production DDL ran.

## Upstream references

- FX selection: https://github.com/QuantumNous/new-api/commit/eb169e74d772afdb281970d0cbe0b62fbf9fa994
- Priority drag ordering: https://github.com/QuantumNous/new-api/commit/5e9595190e3006ecc57712f6a365573b5a20de4d

## Owned-source hashes (updated after the authority correction)

- `services/core/schema/routing_admin.sql`: `421c98a3e3147c647c34694d1787c7bb6a1c624fe8063a17ebfd18935d2d5907`
- `services/core/src/routing_admin/mod.rs`: `79050b5f861494d01474299d1d5e8c8355672e47370677a0ccbd0199bca252c8`
- `services/core/src/routing_admin/plan.rs`: `8fed766f5ea550c87b9e40c9a31a7a2ab64b8e57ffa56316b977c101cb1f9cfd`
- `services/core/src/routing_admin/owner_baseline.rs`: `a507022254e059051c9ba219d2b4ceaefcb75c903dd78172aef18ac10ee2792e`
- `services/core/src/routing_admin/tests.rs`: `70fa3a61f31ab76d7e12d80ddc36ae05a5d0c27f51cd184a2f27867d2e97fd6a`
- `services/core/src/core_schema/routing_admin_upgrade_evidence.rs`: `8f685bcc9a3933013ee99368da53f86d88073f483695e359315ac4a78c3e1c90`
- `services/core/src/runtime_routing_admin.rs`: `4adc2852210383578d9397679524a604d52574d89a1a70c570da7181f4734c91`
- `services/core/src/admin.rs`: `d4316e642245abdf7409e1109e1b2ffbd3b574ac7bc58c3c643b99ce546fca77`
- `services/core/src/main.rs`: `3caa9eb2ae293391843d68b25a4d7b513c6750bd89b6a330c06db2953c421e76`
- `apps/web/src/features/routing-admin/api.test.ts`: `4c2945f6bd33b0d16ea022d9d4579987b6eba16c1a9e78703a521486625c9ab3`
- `apps/web/src/features/routing-admin/api.ts`: `eb5b1b07f501e32b2fa6a63291bea5153bc8f26c0f7df517faeff339b4e779a8`
- `apps/web/src/features/routing-admin/contract.ts`: `03d51ec3d864c3438a71b651449f0950aec1ea7bb1ef571d97861c0e1aa04f59`
- `apps/web/src/features/routing-admin/fixtures.test-helper.ts`: `e7428d7b6af9b97669ae4d16c9cbf904891c5e165ff5f00e64c8aab63fe2e820`
- `apps/web/src/features/routing-admin/helpers.test.ts`: `c2b5d4b87555eea552f535b842fed628d99c440edfadb2901d7c4923eebc17cc`
- `apps/web/src/features/routing-admin/helpers.ts`: `f429f9686a8fab827bb1c0f472e1aa079d2105ae0d034a2b80c6d39de5cc773e`
- `apps/web/src/features/routing-admin/price-sync.tsx`: `fb6d9536c732bd4cfc898b00ca0dc828bc10d7957e219958b17dc109d7c37771`
- `apps/web/src/features/routing-admin/priority-sort.tsx`: `b03769c5433e904a6aa5358bf838d4bf58f7abb716dcc1d8d2a813de3c2d11fb`
- `apps/web/src/features/routing-admin/publication-review.test.tsx`: `7c23ef94846db7275f6c70c06b2bd19924da6ada638bb4bfd7a9511c9164e002`
- `apps/web/src/features/routing-admin/publication-review.tsx`: `f174b98099efaa817f12d71722189e3913772e990f204e3a85a0a1100dc382ef`
- `apps/web/src/features/routing-admin/use-catalog.ts`: `89a0feca0d4dd450f3adc3edd10f938c9e79b0953bf4e3fb3fb719365fa1ecbe`
