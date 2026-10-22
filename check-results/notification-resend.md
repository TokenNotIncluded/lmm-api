# Notification and Resend source checks

This is development evidence, not independent acceptance. No production email,
provider request, database schema mutation, policy activation or subscription
provisioning was executed. Rust compilation, SQLx/PostgreSQL tests, browser
interaction and the full Web typecheck/build remain unrun locally.

## Frozen input

Base HEAD: `451f50b07880051f4c0de43ba99b7c1e2e1b8089` with the coordinated WIP edits. Backend was offered to the root agent
for an immutable CI snapshot; the root snapshot is the authoritative CI input.
The hashes below identify this development readback, not a passing deployment.

## Actual checks

- Rust 1.99 standalone rustfmt, edition 2024, `skip_children=true`, on the 28
  explicitly owned backend files: exit 0. `main.rs`/`config.rs` were formatted
  again after the startup guard. This parses/formats Rust; it does not typecheck.
- `git diff --check`: exit 0 after the backend freeze.
- Native notification helper test bundle: esbuild with external packages and
  the repository `tsconfig.app.json`, followed by
  `node --import ./scripts/test-preload.mjs /tmp/lmm-cache-web-check/notification-preferences.test.js`:
  **7 tests, 7 passed, 0 failed/skipped/cancelled**. Cases cover the six-key
  preference boundary, exact integer threshold validation, owner/session
  binding, stale read rejection, explicit false save, no retry on unknown
  writes and wrong-owner/malformed response rejection. These mock the HTTP
  client; they are not server, database or browser acceptance.
- The existing notification-tab entry and new form dependency graph bundled
  successfully with esbuild. This is syntax/import validation, not full typecheck.
- oxfmt formatted the four notification UI files successfully; the original
  QuantumNous attribution header was saved and restored without changes.
- Twelve new notification strings were applied through the repository
  translation helper for en/zh/zh-TW/fr/ja/ru/vi, followed by i18n sync.
- `python3 -B scripts/check-docs-brand.py --docs-only`: exit 0.

## Rust regressions present but unrun

`notifications/tests.rs` contains ten isolated SQLx PostgreSQL cases: hold
neutrality and capture crossing, top-up rearming, full queue rollback, historical
and same-transaction replay, RepeatableRead rejection, advisory contention
rollback/retry, preference-role/verified-recipient checks, SMTP acceptance
followed by failed acknowledgement without a second connection, and suppression
after unsubscribe or recipient verification removal (several assertions share a
case). Two further restricted-runtime cases cover a missing policy row, later
activation on a node without worker flags, and individual missing publisher
grants causing startup refusal and full money rollback. The fixtures use fresh
SQLx databases and loopback SMTP only.

Pure policy/rendering tests and Resend private-file/receipt/HTTP loopback tests
are also source only until Rust CI executes them. No mocks can replace the
required money transaction/concurrency checks.

## Independent review correction

Root review found that model-only runtime roles previously lacked policy/event
permissions. The final candidate adds a minimal publisher block to the ordinary
runtime grant source and an unconditional startup check in
`runtime_notifications.rs`; full consumer/subscription permissions remain an
optional event-worker capability. Missing privileges are never treated as a
disabled site policy. The shared runtime test role consumes that same marked
grant block. Source formatting/diff checks passed (the routing-admin owner formatted all
three shared permission files after both narrow patches); the two real role tests are
unrun pending PostgreSQL CI. No GRANT was executed locally.

## Operational limits

The durable site policy makes observer/event failures abort the same money
transaction, including finite queue exhaustion. The worker records an attempt
before IO; a crash can lose one email but cannot cause a blind resend. Provider
acceptance does not prove inbox delivery. Only currently verified email is
supported. The balance source is personal wallet + reserved + signed debt,
excluding subscription allowances. Channel/model events reflect committed
routing publications; no automatic channel-health or scheduled supplier-model
discovery task was added. See the module README for activation and caller
isolation details.

## SHA256 readback

| File | SHA256 |
| --- | --- |
| `services/core/src/notifications/mod.rs` | `9a87e9cb97fa5d26445190712ca48d90107ab59748c38c351b11db7f0898ee20` |
| `services/core/src/notifications/policy.rs` | `22dd408e12a51f8ffeaa6d154b7b240c7f0444f6db8a893d4258f25a71d3e23d` |
| `services/core/src/notifications/producer.rs` | `821c1df87a70880a602bd5a6e9528d64a33ae670fe1a8c1c0275a1247a8fcfc3` |
| `services/core/src/notifications/worker.rs` | `cf3db868cd526a4f8885a7945af0a9ea526f5d4694c7e36be2c0bf33fa4adc67` |
| `services/core/src/notifications/tests.rs` | `8d72c84a39eb033ccfbe073517eacc2077733bbac990b7f5880e6170ec6d35d2` |
| `services/core/src/identity/notifications.rs` | `7081147de8a9fd1944b3794aac25970d72df9609d2796a1d2c0950e81843496b` |
| `services/core/src/humangrant/mail.rs` | `537585714dc3e87532637a1432c869b1f63f35d6fa9dd49788b82b80c3fb43f6` |
| `services/core/src/humangrant/mail/resend.rs` | `47181d17b4347be2455da7ccf4248baab3971c469abe57a5b34f63a9f604c96d` |
| `services/core/src/billing/ledger/mod.rs` | `d6cf26f0b9295df26237bab1f8d07cbd916feefdb7229f6063e2b10902fd6b1b` |
| `services/core/src/billing/transactional_ledger.rs` | `87fca46c24feeb2adcebf14f4540b171c82d761037808e4c3e78b156f4e58761` |
| `services/core/src/billing/legacy_escrow.rs` | `98f382251eb99a8d2fcafa69735dfe880d765b8ec277fd83ea16e059a35e979b` |
| `apps/web/src/features/profile/components/notification-preferences.tsx` | `2dc59b0f39cf4409b6ddec6261790f87e071891368eba4ca453d4bf25edb4bcd` |
| `apps/web/src/features/profile/lib/notification-preferences.ts` | `b6fc0b3c0a0e9b289126d81ceba37d5a9a3a63fc7c89c330599752e354fb7d13` |
| `apps/web/src/features/profile/lib/notification-preferences.test.ts` | `28d2e9de7b7ada9b9c506e69465adc6c898291c76a531294e559ff6215b480e3` |
| `apps/web/src/features/profile/components/tabs/notification-tab.tsx` | `4fdfabb2fa8ad41e44ad715e9f96aa1482a38002197763919f4aba432183a500` |
| `services/core/src/runtime_notifications.rs` | `49d05a5347cc6c69c9c2bc5d85e7cc8d5422552fb9af9621a8b0b624660cf961` |
| `services/core/src/runtime_database.rs` | `b56f0d2d268d484167fe359afbe1779f4ce0cb0aa01ee94a3c00eb284c01d851` |
| `services/core/tests/common/runtime.rs` | `7b44a8728b5afb1810eaf41e32523b4c5be6a5e4900ba503ad1f841974898198` |
| `deployment/postgres/grant-core-runtime.sql` | `ebf66cb4bea3614086cc6ea073b6698632236c6934eeb34d2aa2cf29d37816ef` |
