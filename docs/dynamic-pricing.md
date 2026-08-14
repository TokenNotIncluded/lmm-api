# Dynamic pricing

Dynamic pricing continuously adjusts the effective price factor of a model
based on recent load, upstream cost, and the rate at which the factor is
allowed to move. It is a **multiplier on top of the configured price** — it
never replaces the base pricing (model ratio / model price), it scales it.

The feature is implemented in three layers:

- `pkg/dynamic_pricing` — pure engine: the formula pipeline (cost floor +
  load premium) and the in-memory per-model state store with best-effort
  Redis persistence.
- `service` — a background ticker that periodically aggregates consume logs
  over a sliding window and feeds them into the engine.
- `setting/dynamic_pricing_setting` — all configuration, stored as
  `dynamic_pricing_setting.*` DB keys.

## Overview

At a high level the factor is the product of two ideas:

1. **Cost floor.** The multiplier never lets the effective price undercut
   `cost_floor_factor × upstream cost`. The anchor cost `C` is the larger of
   the expected route cost (cheap channel + failover premium) and the smoothed
   measured unit cost of recent traffic, then scaled by `cost_floor_factor`.
2. **Supply–demand premium.** Excess load above configured targets (tokens,
   requests, or upstream cost rate) raises the multiplier, shaped by a
   deadzone and a non-linear exponent, capped at `max_factor`, and moved with
   an asymmetric EMA plus per-tick step clamps.

The multiplier is read once per request on the billing path (see
[Pricing integration](#known-limitations--out-of-scope)) and applied as an extra ratio
(`dynamic_pricing`) on top of the normal model price. Revenue is deliberately
never used as a load input (see [Design rationale](#design-rationale)).

The feature is disabled by default (`dynamic_pricing_setting.enabled =
false`); the tick is a no-op while disabled, re-checked each tick, so toggling
does not require a restart.

## Formula pipeline

Each tick (`pkg/dynamic_pricing.Tick`) evolves one model's state through the
following steps, exactly in this order:

1. **Route cost** — the expected upstream unit cost given failover routing:

   ```
   C_route = cheap + p_failover × (backup − cheap)
   ```

   where `p_failover` is clamped to `[0,1]`. If the backup cost is unknown
   (`<= 0`) or not more expensive than the cheap route, `C_route = cheap`.
   (`cheap` = cheapest configured channel cost among the model's channels in
   the window, `backup` = second-cheapest.)

2. **Cost EMA** — the measured actual unit cost (USD per 1M tokens,
   `costUSD × 1e6 / tokens`) is smoothed with an EMA using `alpha_load`. If
   the window carried no tokens, the previous EMA is kept.

3. **Effective cost** — the anchor for the factor, never below the configured
   cost floor:

   ```
   C = max(C_route, EMA(actual unit cost)) × cost_floor_factor
   ```

   Costs are USD per 1M tokens. `C` is compared with the model's configured
   `base_price_usd_per_million` to produce a price multiplier.

4. **Raw load** — how far the window exceeded the targets, the maximum over
   the dimensions whose target is positive:

   ```
   L_raw = max( TPM/t / target_tpm , RPM/t / target_rpm , costRate/t / target_cost_rate )
   ```

   `1.0` means exactly at target; if no target is positive, or the window
   length is not positive, `L_raw = 0` (no load signal).

5. **Load EMA** — `L = EMA(L_raw)` with `alpha_load`. Cold start (`prev == 0`)
   takes the raw sample directly instead of decaying from zero.

6. **Heat** — maps excess load to `[0,1]`:

   ```
   heat = clamp((L − deadzone) / (1 − deadzone), 0, 1) ^ gamma
   ```

   Load at or below the deadzone produces no heat. Invalid deadzones
   (outside `[0,1)`) default to 0; `gamma < 1` defaults to 1.

7. **Target multiplier**:

   ```
   cost_factor = C / base_price_usd_per_million
   load_factor = 1 + (max_factor − 1) × heat
   target = clamp(max(cost_factor, load_factor), 1, max_factor)
   ```

   In `[1, max_factor]`. If the effective cost is unknown/not positive, the
   target is `1.0` (base price). A zero base-price setting uses the safe
   reference value `$1 / 1M tokens` for backwards compatibility.

8. **Asymmetric EMA** toward the target — `alpha_up` when the factor must
   rise, `alpha_down` when it must fall. As a pure function a non-positive
   `prev` returns the target directly; inside `Tick` the cold-start guard in
   step 9's note has already normalized `Factor` to `1.0` by this point.

9. **Bounds** — per-tick step clamps (`mult ≤ prev×(1+max_step_up)`,
   `mult ≥ prev×(1−max_step_down)`) and the absolute range `[1, max_factor]`.
   Unknown/non-positive cost yields `1.0`. A cold start seeds `Factor = 1.0`
   (and `Tick` defensively treats `Factor ≤ 0` as `1.0`), so the step-up
   clamp binds from the very first tick — a hot first window can never jump
   straight to `max_factor`; it climbs at most `+max_step_up` per tick.
   (`EnforceBounds` still skips the clamp when called as a pure function
   with a non-positive `prev`.)

**No-cost-signal tick.** When a non-empty tick carries no upstream cost
information (`cheap ≤ 0`, `backup ≤ 0`, and window cost `≤ 0`) — e.g. the
admin removed the `channel_costs` entries mid-run — the model is treated as
having lost its cost configuration: `Tick` resets `CostEMA = 0` and
`Factor = 1.0`, sets `updated_at`, and returns `1.0` immediately. A stale warm
`CostEMA` must never keep the multiplier elevated once there is no cost signal
to anchor it, so prices fall back to the configured base.

For a genuinely idle model, the ticker sends an explicit zero-load sample
instead of resetting it. Its load/cost EMAs and factor then decay gradually
toward the base price, so a high factor cannot persist after traffic stops.

The persisted state per model is `{load_ema, cost_ema, factor, updated_at}`.

## Design rationale

**Revenue is never a load input.** Charging more under load would feed back
into measured revenue, and a higher revenue measurement would push the factor
higher still — a positive feedback loop that could spiral. Only token/request
volume and upstream cost drive the factor. This is a hard design constraint,
not a tunable.

**Cost uses configured channel costs, never billed amounts.** Upstream cost
is derived from the admin-configured `channel_costs` map (USD per 1M tokens
per channel) in `dynamic_pricing_setting`, multiplied by token volume. Billed
quota amounts are never used, because they already include the multiplier
itself (billing is downstream of pricing) and would make the cost signal
self-referential. Channels without a configured cost are excluded from the
cost calculation and from cheap/backup route selection.

**Why a deadzone + gamma + asymmetric EMA + step clamps.** The deadzone stops
small fluctuations from churning the factor; `gamma > 1` keeps small excesses
gentle and amplifies large ones; the asymmetric EMA rises fast (`alpha_up`)
but falls slowly (`alpha_down`) — protecting margin quickly while avoiding
pricing whiplash on relief; step clamps bound how much the factor can move in
a single tick so prices cannot jump discontinuously.

## Configuration

All settings live in the `dynamic_pricing_setting` config module, so every DB
key is `dynamic_pricing_setting.<field>` (for example
`dynamic_pricing_setting.enabled`, `dynamic_pricing_setting.target_tpm`).

| DB key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `dynamic_pricing_setting.enabled` | bool | `false` | Master switch. The tick is a no-op while disabled (re-checked per tick, no restart needed). |
| `dynamic_pricing_setting.tick_interval_seconds` | int | `60` | Sleep between ticks. Re-read every iteration; it must be positive. An invalid configuration sleeps at the 60 s fallback but skips the tick. |
| `dynamic_pricing_setting.window_minutes` | int | `5` | Sliding window length; consume logs in `[now−window, now]` are aggregated per tick. Must be positive or the tick is skipped. |
| `dynamic_pricing_setting.target_tpm` | float | `100000` | Tokens-per-minute target; one load dimension. |
| `dynamic_pricing_setting.target_rpm` | float | `60` | Requests-per-minute target; one load dimension. |
| `dynamic_pricing_setting.target_cost_rate` | float | `1.0` | Upstream-cost rate target, USD per minute; one load dimension. |
| `dynamic_pricing_setting.base_price_usd_per_million` | float | `1.0` | Reference price in USD per 1M tokens used to convert the upstream cost floor into a multiplier. Zero falls back to `1.0`. |
| `dynamic_pricing_setting.alpha_load` | float | `0.3` | EMA smoothing for raw load (and for the measured unit cost). First sample is taken directly (cold start). |
| `dynamic_pricing_setting.alpha_up` | float | `0.30` | Asymmetric-EMA alpha used when the factor must rise. |
| `dynamic_pricing_setting.alpha_down` | float | `0.05` | Asymmetric-EMA alpha used when the factor must fall. |
| `dynamic_pricing_setting.cost_floor_factor` | float | `1.2` | Multiplies the effective upstream cost before it is compared with the model base price. Must be finite and at least `1`. |
| `dynamic_pricing_setting.max_factor` | float | `3.0` | Absolute ceiling for the multiplier; the absolute range is `[1, max_factor]`. |
| `dynamic_pricing_setting.load_deadzone` | float | `0.4` | Load at or below this fraction of target produces no heat. It must be finite and in `[0,1)`. |
| `dynamic_pricing_setting.heat_gamma` | float | `2.0` | Non-linear shaping exponent on heat; it must be finite and at least `1`. |
| `dynamic_pricing_setting.max_step_up` | float | `0.10` | Per-tick upward clamp: `mult ≤ prev × (1 + 0.10)`. |
| `dynamic_pricing_setting.max_step_down` | float | `0.03` | Per-tick downward clamp: `mult ≥ prev × (1 − 0.03)`. |
| `dynamic_pricing_setting.failover_probability` | float | `0.15` | Expected probability of failing over to the backup channel, used in the route cost; it must be finite and in `[0,1]`. No effect when the backup cost is unknown or ≤ cheap. |
| `dynamic_pricing_setting.channel_costs` | map (JSON) | `{}` | Upstream cost in USD per 1M tokens keyed by channel ID (string keys, e.g. `{"12":1.8}`). Empty means no costs configured. |
| `dynamic_pricing_setting.per_model` | map (JSON) | `{}` | Per-model overrides of the three load targets and optional `base_price_usd_per_million`, e.g. `{"gpt-5":{"target_tpm":50000,"base_price_usd_per_million":8}}`. A zero override field (or an absent model) inherits the global value. |

### How keys are applied

Keys are written through the standard admin options API
(`PUT /api/option`, root auth) as `dynamic_pricing_setting.<field>` and stored
in `OptionMap`. The update path then routes every `*.*` key through
`handleConfigUpdate`, which splits on the first `.`, resolves the config
module name, and applies the field via `config.UpdateConfigFromMap` (matching
each field's JSON tag). Because `dynamic_pricing_setting` is a registered
config module, **all of its keys flow through automatically** — no code change
or restart is required. The ticker re-reads the setting on every iteration, so
changes take effect on the next tick. Map fields (`channel_costs`,
`per_model`) are set as JSON strings.

Two caveats on the update plumbing:

- Unlike `billing_setting.*` keys, `dynamic_pricing_setting.*` keys are **not**
  matched by `IsPricingOptionKey` (no pricing-cache invalidation) and have no
  dedicated post-processing hook in `handleConfigUpdate`. This is harmless for
  the multiplier itself (it is read live per request, not cached), but there
  is no generic settings UI guaranteed to render these fields — the API is the
  integration point.
- Invalid numeric controls, restored factors outside `[1, max_factor]`, and
  non-finite channel costs fail closed: the tick is skipped or the state is
  reset to a safe neutral value.

## Status API

`GET /api/dynamic_pricing/status` (admin auth) returns a read-only snapshot:

```json
{
  "success": true,
  "message": "",
  "data": {
    "enabled": false,
    "setting": {
      "enabled": false,
      "tick_interval_seconds": 60,
      "window_minutes": 5,
      "target_tpm": 100000,
      "target_rpm": 60,
      "target_cost_rate": 1.0,
      "base_price_usd_per_million": 1.0,
      "alpha_load": 0.3,
      "alpha_up": 0.30,
      "alpha_down": 0.05,
      "cost_floor_factor": 1.2,
      "max_factor": 3.0,
      "load_deadzone": 0.4,
      "heat_gamma": 2.0,
      "max_step_up": 0.10,
      "max_step_down": 0.03,
      "failover_probability": 0.15,
      "channel_costs": {},
      "per_model": {}
    },
    "models": {
      "gpt-5": {
        "factor": 1.5,
        "load_ema": 0.7,
        "cost_ema": 1.12,
        "updated_at": 1786000000
      }
    }
  }
}
```

- `setting` is the full `DynamicPricingSetting` (no sensitive fields).
- `models` contains only models with current in-memory state (models that were
  ticked at least once since start, or whose state was loaded from Redis).
  Each entry exposes `factor` (current multiplier), `load_ema`, `cost_ema`,
  and `updated_at` (unix seconds of the last tick).

## Operational notes

**Redis is optional.** Per-model state lives in an in-memory map; Redis
(active only when Redis is enabled and the client is live) is a best-effort
persistence layer with a 24 h TTL, written on every state change and read
back only on a ticker cold start. Every Redis failure is logged and swallowed:
Redis being down degrades to in-memory-only pricing, never to an error on the
request path. **Multi-instance caveat:** each node runs its own ticker and
keeps its own in-memory state, and the request path reads the local multiplier
(no per-request Redis). Factors can therefore drift between nodes; Redis only
bridges a single node's cold start.

**Log-aggregation DB load.** Every tick issues one aggregation over the
consume-log table (`type = 2`) for `[now−window, now]`, grouped by
`model_name, channel_id` — i.e. a full-window scan per tick, per node. On
large deployments tune `window_minutes` and `tick_interval_seconds` (or
disable the feature) to bound query cost. A lighter bucketed aggregation
exists (`perf_metrics` table via `perf_metrics_setting`), but the dynamic
pricing ticker currently reads the raw consume-log table only; wiring it to a
dedicated aggregation source is future work.

**Mixed channel windows use priced traffic only.** Channels without a
`channel_costs` entry (or with cost `≤ 0`) are excluded from the priced token
and request denominators, upstream-cost calculation, and cheap/backup route
selection. Their traffic is tracked separately. This prevents a large
unconfigured channel from diluting the cost rate calculated from configured
channels. If a window contains only unpriced traffic, the model falls back to
the base price.

**Zero-traffic models decay.** The ticker iterates models already held in
memory even when the current window has no consume rows. Each such model gets
a zero-load sample, so its factor and EMAs move down under the configured
step/EMA controls. Redis state is also clamped to the current absolute factor
range when restored.

**Load above target only warns.** When `load_ema > 1.0` the ticker logs a
warning; dynamic pricing never rejects or sheds traffic (see limitations).

## Known limitations / out of scope

- **Not capacity control.** Exceeding the load targets only raises the price
  factor (and logs a warning). Dynamic pricing cannot throttle, queue, or
  reject requests — it is a pricing signal, and cannot replace capacity
  control.
- **No frontend panel yet.** There is no admin UI for this feature; the
  status API (`/api/dynamic_pricing/status`) is the integration point, and
  configuration is done through the options API / settings keys.
- **Pricing integration.** The multiplier is captured as an extra
  `dynamic_pricing` ratio for fixed-price, ratio, per-call, and tiered
  pre-consume paths. Audio/realtime settlement and tiered post-settlement use
  the same captured ratio. Task submit-time ratio replacement merges the
  captured dynamic ratio instead of dropping it.
- **Per-model overrides cover the three load targets and the reference base
  price**; the shaping
  parameters (deadzone, gamma, alphas, clamps, `max_factor`,
  `failover_probability`) are global.

## Worked example

Assume a model routed through two channels with configured costs
`cheap = 1.0`, `backup = 1.8` (USD per 1M tokens) and the default
`failover_probability = 0.15`:

```
C_route = 1.0 + 0.15 × (1.8 − 1.0) = 1.12
```

Assume the smoothed measured unit cost stays at or below `1.12`. With the
default `cost_floor_factor = 1.2`, `base_price_usd_per_million = 1.0`,
`max_factor = 3.0`, `load_deadzone = 0.4`, and `heat_gamma = 2.0`, the
effective cost is `C = 1.344` and the steady-state multiplier is
`max(1.344, 1 + (3−1) × heat)` at a given load `L`:

| Load `L` | `x = (L − 0.4) / (1 − 0.4)` | `heat = x²` | Multiplier |
| --- | --- | --- | --- |
| 40% (0.4) | 0 | 0 | **1.34×** |
| 70% (0.7) | 0.5 | 0.25 | **1.50×** |
| 90% (0.9) | 0.833… | 0.694… | **2.39×** |
| 100% (1.0) | 1 | 1 | **3.00×** |

Notes on the example:

- These are steady-state values: the asymmetric EMA converges to the target,
  and the per-tick step clamps bound only the *rate* of movement. With the
  defaults, a sudden jump from `1.0×` toward `3.0×` climbs roughly `+10%` per
  tick (the step clamp binds early), while relief falls at most `3%` per tick
  (and the `alpha_down = 0.05` EMA slows it further).
- Earlier planning notes quoted `~1.65×` at 70% and `~2.45×` at 90%; those
  figures assume a slightly lighter shaping (e.g. `load_deadzone ≈ 0.3` or
  `heat_gamma ≈ 1.7`). The exact multiplier is fully determined by the
  formulas in [Formula pipeline](#formula-pipeline) with whatever parameters
  are configured — there is no other shaping.
- The multiplier applies on top of the configured price: a model priced at
  `$10 / 1M` tokens charges `$10 × factor` (rounded to quota units) while the
  feature is active for it.
