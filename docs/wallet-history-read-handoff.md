# Wallet history read handoff

The self endpoint reads only the owner-imported `wallet-topup-history.v1`
receipt. The receipt is an immutable `core_migration.import_runs` row; it does
not add a table, alter the schema catalog, create orders, or write any balance,
credit, payment intent, journal, or counter.

The old `top_ups` table is the order source. `core_history.log_records` with
`log_type=1` is a separate request-history source, not a matching order list;
158 such logs cannot be subtracted from 322 top-ups to claim 164 missing
orders. Native `core_payments.intents` are outside this legacy snapshot and
are reported separately by verification.

## Export and prepare

Run the export query only against an offline restore of the named legacy
database and schema. Do not connect it to a production primary:

```sh
psql --no-psqlrc --quiet --tuples-only --no-align --set ON_ERROR_STOP=1 \
  --file scripts/export_wallet_history_snapshot.sql \
  "postgresql://<offline-restore>" > /secure/path/topups.json
```

Supply the identity map from the reviewed financial migration material. The
map must contain every legacy user in the export, including users with zero
orders, with each row shaped as:

```json
{"source_user_id": 7, "target_user_id": 71, "target_personal_account_id": 701}
```

Prepare into a new private directory. The archive and export are hashed and
bound into the plan; `--expected-rows 322` makes the independently observed
source count a required gate:

```sh
umask 077
python3 scripts/prepare_wallet_history_snapshot.py \
  --source-export /secure/path/topups.json \
  --identity-map /secure/path/identity-map.json \
  --source-archive /secure/path/lmm_api.dump \
  --expected-rows 322 \
  --output /secure/path/wallet-topup-history-import
```

Review `plan.json` and `manifest.json` before applying. A count mismatch,
duplicate order, unknown status, invalid amount or missing identity mapping
blocks generation of `apply.sql`. Preserve `blocked.json` as gap evidence.
Money is retained as exact decimal text; unknown currency stays unknown.
Currency totals group unknown currency separately and never convert to USD.

## Owner apply and verify

The offline installation owner, using the same owner role that owns
`core_migration.import_runs`, applies the generated transaction once:

```sh
psql --no-psqlrc --set ON_ERROR_STOP=1 \
  "postgresql://<offline-target-owner>" \
  --file /secure/path/wallet-topup-history-import/apply.sql
psql --no-psqlrc --quiet --tuples-only --no-align --set ON_ERROR_STOP=1 \
  "postgresql://<offline-target-owner>" \
  --file /secure/path/wallet-topup-history-import/verify.sql
```

The transaction takes the Core installer advisory lock, checks that every
mapped target user still owns the expected personal account, rejects a
different installed snapshot, and verifies exact JSONB equality when reading the
inserted immutable manifest back. Rerunning the same prepared snapshot is idempotent;
changing its archive, rows, or mapping under the same migration ID fails.
`verify.sql` reports source/import/missing counts, status and currency totals,
plus independent current counts of type-1 history rows and native payment
intents. It expects no equation between those sources.

For live reads, the runtime role needs only read access to the existing
immutable receipt:

```sql
GRANT USAGE ON SCHEMA core_migration TO lmm_core_runtime;
GRANT SELECT (migration_id, manifest)
  ON core_migration.import_runs TO lmm_core_runtime;
```

Run the existing database privilege check after applying grants. The query
uses the authenticated session's target user and personal account. A user
listed in the export with zero orders gets an evidence-backed empty page; a
user absent from the imported mapping gets `503` rather than an invented
empty history. Responses identify the source snapshot and its imported/missing
counts, and explicitly state that later native payment orders are not included.

Main-agent source wiring after review:

```rust
pub mod wallet_topup_snapshot;
pub mod wallet_history_read_http;
```

At router construction, configure the fixed migration ID from the accepted
manifest and attach the same Core pool:

```rust
let wallet = wallet_history_read_http::WalletHistoryReadHttp::new(identity.clone())
    .with_history(wallet_topup_snapshot::WalletTopupHistory::new(
        pool.clone(),
        accepted_wallet_history_migration_id,
    ));
let router = router.merge(wallet_history_read_http::router(wallet));
```

The accepted ID must come from reviewed deployment configuration, not a query
parameter. This work does not apply the archive, authorize live database
changes, or imply that production history has been imported.

## Compatibility and ownership

The port targets WIP HEAD `2909d9a79c58767b1fcbac940069bd827731a137`.
It uses the existing `IdentityStore::authorize` session check,
`Principal::owner_account_id`, `core_identity.users.personal_account_id`, and
immutable `core_migration.import_runs` receipt contract. There are no Core schema
changes. The generated import checks the mapped target identities again at apply
time; identities must already exist from the accepted identity migration.

Merge the history router after constructing the main stateless router, alongside
other `Router` modules. This module exclusively registers
`GET /api/user/topup/self`; it has no startup database writes. The installation
owner must accept/import the immutable snapshot before wiring a history reader.
Do not infer an import ID from any request input.

The separate recharge configuration owner retains `/api/user/topup/info`. This
module does not register `/api/user/topup`, the admin list endpoint used by the
current frontend when `isAdmin` is true. It also does not read native payment
orders, activate payments, reconstruct log rows as orders, or change funds.
The frontend contract keeps `money` numeric, original decimal text in
`money_text`, original order IDs/trade numbers/status/timestamps, and remapped
`user_id`. Personal history responses include explicit snapshot coverage.

Directed self-tests compile these unwired modules by path, without modifying
`http.rs`, `main.rs`, or `lib.rs`:

```sh
CARGO_TARGET_DIR=/private/task-target CARGO_HOME=/private/task-cargo \
  DATABASE_URL='postgresql://<isolated-local-test-owner>' \
  cargo test --manifest-path services/core/Cargo.toml --locked --offline \
  --test wallet_history_read_http
python3 scripts/test_prepare_wallet_history_snapshot.py
```

The PostgreSQL test creates only isolated test databases. It exercises generated
import SQL, immutable retry, runtime SELECT permissions, exact decimal output,
all four known statuses, personal user/account isolation, pagination/search/sort,
zero-order versus uncovered users, revoked sessions, API key rejection, and
complete business table/sequence snapshots before and after reads/import retries.
Self-tested; independent acceptance not run. No live database read/import,
commit, push, or deployment is part of this handoff.
