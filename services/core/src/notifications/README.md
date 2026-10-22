# Notification preferences, committed facts and email delivery

This module owns three optional notifications: a personal wallet crossing its
low-balance threshold, a committed channel configuration change, and a change
to the configured channel/model bindings. The preference names follow
[upstream notification controls](https://github.com/QuantumNous/new-api/commit/6461f084ffcd).
Mail providers include SMTP and the
[upstream Resend addition](https://github.com/QuantumNous/new-api/commit/32e587406c6ebe891f18ab9aeb2f4a1f6b7b9e14).
The Rust implementation uses existing Core identity, ledger and event storage.
No JavaScript runtime, Go process or new database schema is required.

This is a source candidate. The local checkout used for development did not
have Cargo or PostgreSQL; the Rust/SQLx tests still require execution in the
normal isolated CI database before acceptance. See the actual check record in
[notification-resend.md](../../../../check-results/notification-resend.md).

## Preferences and scope

`GET /api/user/notifications` and `PUT /api/user/notifications` require the
current native browser session. The returned `user_id` belongs to that session.
The write accepts only these keys:

| Key | Meaning |
| --- | --- |
| `quota_warning_enabled` | Receive a real downward wallet threshold crossing; absent legacy preference defaults to enabled. |
| `quota_warning_threshold` | Integer `credit_500k_usd` units; `0` uses the site default. Maximum is `9007199254740991`. |
| `channel_notify_enabled` | Administrative channel publication notification. Actual `superadmin` defaults to enabled; actual `admin` defaults to disabled. |
| `upstream_model_update_notify_enabled` | Administrative configured model-binding notification; defaults to disabled. |
| `notify_type` | The native sender accepts `email`. A previously stored other method is retained on read and suppresses mail until an explicit email save. |
| `notification_email` | Empty follows the currently verified account email. A nonempty override must match that same verified address. |

Ordinary users cannot enable either administrative notification. A service or
trust level is never administrative authority. The locked profile JSON is
merged, preserving unrelated preferences. Unknown write keys are rejected;
this route grants no price, key, IP-policy or financial permission.

The worker rechecks the current active user/account, current platform role,
current preferences and `email_login_allowed` immediately before recording a
delivery attempt. It never falls back from an old webhook or an unverified
override. Email text uses the recipient's current supported language. Event
payloads hold counts and version/balance facts, without email addresses,
credentials, model responses or complete routing documents.

## Explicit activation

Ordinary service startup only validates configured resources. It cannot create
the subscription, enable site policy, install schema or send a test email.
Use the separate installation-owner configuration for the explicit offline
`configure-notifications` operation, after reviewing the candidate and existing
runtime grants. No command below was executed during development.

The command reads at most 4096 bytes of JSON from stdin. This example creates
the first policy; an update must supply the exact current **option revision**
in `expected_revision` instead of `null`:

```json
{
  "expected_revision": null,
  "policy": {
    "revision": 1,
    "quota_warning_enabled": true,
    "quota_warning_threshold": 500000,
    "channel_notify_enabled": true,
    "upstream_model_update_notify_enabled": true
  }
}
```

```sh
lmm-core --config /absolute/private/owner.toml configure-notifications < /absolute/private/notification-policy.json
```

This stores `CoreNotificationsPolicyV1` in `core_console.options` and provisions
the existing `core_events` consumer `core-notifications-v1`, service
`core-notifications`, for `notifications.balance.v1`, `notifications.channel.v1`
and `notifications.models.v1`, with at most 10000 pending deliveries. The
operation requires owner write privileges and compares the previous revision
in the same serializable transaction. It changes no balances and prints only
the new revision and `mail_sent: false`.

The durable policy activates all money/routing publishers on every node,
including nodes without a local mail worker. Configure the worker on at least
one serving node; multiple nodes share event leases and the durable attempt
receipt.

All database runtimes require the base notification-publisher grant block in
`deployment/postgres/grant-core-runtime.sql`, regardless of the policy's current
value or local worker configuration. Startup verifies read access to the site
policy and the minimum event append/inbox columns. These grants cannot edit
the policy, create subscriptions or acknowledge deliveries. Worker nodes also
require the existing `enable_events=true` consumer grants. Apply the reviewed
runtime grant changes and restart every money-writing node before enabling the
durable policy; do not treat a missing grant as a disabled policy. Existing-site
grant application is a separate owner operation, never ordinary startup.

Core configuration also requires its database and native Web origin:

```toml
[notifications]
config_file = "/etc/lmm.conf.d/core-notifications.toml"
```

The referenced private TOML selects one provider. For Resend:

```toml
[mail]
provider = "resend"
from = "notifications@example.invalid"
from_name = "LMM Forge"
api_key_file = "/etc/lmm.conf.d/secrets/core/resend-api-key"
```

For SMTP:

```toml
[mail]
provider = "smtp"
host = "smtp.example.invalid"
port = 587
from = "notifications@example.invalid"
tls = "starttls"
username = "notifications@example.invalid"
password_file = "/etc/lmm.conf.d/secrets/core/smtp-password"
```

These addresses and hosts are documentation placeholders, not test recipients.
Use private regular configuration/key files with permissions `0600` or `0400`;
symlinks, public secret files and parent-directory traversal are rejected.
Resend always uses `https://api.resend.com/emails`, with verified TLS, no proxy,
no redirects, no HTTP retry and a 20-second total deadline. The bounded success
response must contain a valid nonempty receipt ID. It sends plain text with a
stable `Idempotency-Key`; the provider's retention window does not authorize a
retry after an unknown send. See the
[official send API](https://resend.com/docs/api-reference/emails/send-email) and
[idempotency contract](https://resend.com/docs/dashboard/emails/idempotency-keys).

HumanGrant can select the same provider implementation using
`identity.humangrant_mail_file` in place of `identity.humangrant_smtp_file`.
That file contains the provider fields directly, without a `[mail]` table.
The old SMTP file is still accepted. Configuring both files or configuring mail
while HumanGrant is disabled is rejected. Notification delivery and HumanGrant
retain their separate durable queues and explicit configuration.

## Money transaction ordering and failure behavior

The canonical ledger wrapper invokes the observer after the real ledger
function and before the caller commits. It reads the immutable journal's net
change to `wallet + reserved + debt`, where debt is signed. Moving funds into
or out of a hold does not change this economic balance. Affiliate entitlement
and subscription allowance are outside this notification's scope.

Only a negative change satisfying `before >= threshold && after < threshold`
produces an event. A later real credit can rearm a new crossing; another charge
while already below the threshold does not produce another warning. Replaying
a historical operation cannot derive an old warning from today's balance.
An inbox observation receipt also guards repeated use of a journal inside the
same transaction, including a journal that originally produced no warning.

Observers acquire transaction advisory locks for all affected accounts in
ascending account order. `pg_try_advisory_xact_lock` never waits behind an
observer that may itself need a ledger bucket lock. `READ COMMITTED` reads the
latest committed aggregate after acquiring the observer lock; `SERIALIZABLE`
transactions retain PostgreSQL's serialization conflict detection. The observer
rejects `REPEATABLE READ`, which cannot safely infer the preceding economic
balance after concurrent changes to different buckets.

| Caller | Existing transaction boundary |
| --- | --- |
| `billing::ledger::Ledger::execute` | Explicit `READ COMMITTED`. |
| Charging's `transactional_ledger` | Charging's existing serializable transaction. |
| Payments and commerce | Existing serializable business transaction. |
| Wallet and gift | Existing default read-committed transaction, or the enclosing business transaction. |
| Bounties | Existing serializable transaction. |
| Tool market, including credit execution | Existing serializable transaction. |
| Legacy escrow finalization | Existing serializable finalization transaction. |

All canonical Rust `post_ledger` callers use the wrapper; legacy escrow observes
its separately returned journal before commit. A lock conflict, malformed
policy, missing evidence, full queue or event write failure propagates out of
the enclosing transaction. The money write and event either both commit or
both roll back. Existing callers retain their operation keys and recovery
rules; an unknown COMMIT is never blindly replayed with a new identity.

This creates an explicit availability tradeoff after activation: if delivery
stops long enough to fill the finite queue, a transaction that needs a new
notification cannot commit. The implementation does not silently drop facts
to keep accepting money writes. Disabling a notification kind via the explicit
site policy stops producing that kind; already queued facts are suppressed by
the worker's current-policy check. Do not disable a worker before considering
its pending queue.

## Delivery and routing facts

The worker takes one leased event at a time. It validates the payload, event
fingerprint, resource owner and journal/configuration version. After current
authorization and preferences are checked, it commits an immutable attempt
receipt **before** network IO. A receipt with `attempt_started` prevents every
later attempt from opening another connection. A suppressed receipt remains
suppressed on replay. Result recording and event acknowledgement share a
transaction.

Provider acceptance is recorded as `accepted`; a timeout, explicit rejection,
crash between receipt and IO, or uncertain outcome is `unknown`. This is an
at-most-once attempt contract, so a process crash in that interval can lose an
email. It does not promise exactly-once delivery or inbox receipt. There is no
SMTP-to-Resend fallback or automatic resend after an unknown result.

Both ordinary routing publication and native routing-admin publication append
their facts in the same transaction as the new routing release. A price-only
release does not trigger a channel event. Publication replays retain their
idempotent operation and event identity. The implementation covers configured
channel and model changes; it does not add a scheduled supplier-model discovery
service or automatically disable channels based on health probes.

## Verification

`policy.rs`, `worker.rs`, `tests.rs` and `humangrant/mail/resend.rs` contain the
pure, isolated PostgreSQL and loopback-transport regressions. The PostgreSQL
cases cover holds, capture crossings, credit rearming, historical and
same-transaction replays, observer contention, unsupported isolation, capacity
rollback, real role and verified-recipient checks, suppression after preference
change, and no second SMTP connection after acknowledgement failure. Restricted
runtime cases cover an absent policy row, later activation without a local
worker, and missing individual publisher grants rejecting startup and rolling
back the same money operation. They must run with the normal isolated SQLx
test database. No production database,
provider key or external mailbox belongs in these tests.
