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

At a high level the factor is the maximum of three safeguards/signals:

1. **Operator minimum.** While enabled, the request multiplier is never below
   `min_factor`, including before the first model sample exists.
2. **Cost floor.** The multiplier never lets the effective price undercut
   `cost_floor_factor × upstream cost`. The anchor cost `C` is the larger of
   the expected route cost, the current measured unit cost, and the smoothed
   measured unit cost, then scaled by `cost_floor_factor`. This floor is
   immediate and may exceed `max_factor`; smoothing and a demand-price ceiling
   are never allowed to force a known loss.
3. **Supply–demand premium.** Excess load above configured targets (tokens,
   requests, or upstream cost rate) raises the multiplier, shaped by a
   deadzone and a non-linear exponent, capped at `max_factor`, and moved with
   an asymmetric EMA plus per-tick step clamps.

The multiplier is captured on the billing path and revalidated against the
selected channel before every upstream attempt (including retries). It is
applied as an extra ratio (`dynamic_pricing`) on top of the normal model
price. Revenue is deliberately never used as a load input (see
[Design rationale](#design-rationale)).

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

3. **Effective cost** — the anchor for the factor. A current cost increase is
   used immediately instead of waiting for its EMA to converge:

   ```
   C = max(C_route, current actual unit cost, EMA(actual unit cost)) × cost_floor_factor
   hard_cost_factor = C / base_price_usd_per_million
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

7. **Demand target multiplier**:

   ```
   cost_factor = C / base_price_usd_per_million
   load_factor = 1 + (max_factor − 1) × heat
   smoothed_target = clamp(max(cost_factor, load_factor), 1, max_factor)
   ```

   The demand target is in `[1, max_factor]`. Enabling the feature requires a
   positive reference base price. If cost is unknown, the engine target falls
   back to `min_factor`, while the strict request path blocks an unknown-cost
   selected channel before upstream spend.

8. **Asymmetric EMA** toward the target — `alpha_up` when the factor must
   rise, `alpha_down` when it must fall. As a pure function a non-positive
   `prev` returns the target directly; inside `Tick` the cold-start guard in
   step 9's note has already normalized `Factor` to `1.0` by this point.

9. **Bounds and hard floors** — per-tick step clamps
   (`mult ≤ prev×(1+max_step_up)`, `mult ≥ prev×(1−max_step_down)`) and
   `[1, max_factor]` apply to the demand target. The final value is:

   ```
   factor = max(min_factor, hard_cost_factor, smoothed_demand_factor)
   ```

   Consequently a load-only increase remains smooth, while a known cost
   increase takes effect immediately and may exceed both the movement clamp
   and `max_factor`.

**No-cost-signal tick.** When a non-empty tick carries no upstream cost
information (`cheap ≤ 0`, `backup ≤ 0`, and window cost `≤ 0`) — e.g. the
admin removed the `channel_costs` entries mid-run — the model is treated as
having lost its cost configuration: `Tick` resets `CostEMA = 0` and
`Factor = min_factor`, sets `updated_at`, and returns the configured minimum
immediately. A stale warm
`CostEMA` must never keep the multiplier elevated once there is no cost signal
to anchor it. The request-path unknown-cost guard remains authoritative.

For a genuinely idle model, the ticker sends an explicit zero-load sample
instead of resetting it. Its load/cost EMAs and factor then decay gradually
toward `min_factor`, so a high factor cannot persist after traffic stops.

The persisted state also records the latest hard cost floor and unknown-cost
traffic counters for the live safety preview.

## Design rationale

**Revenue is never a load input.** Charging more under load would feed back
into measured revenue, and a higher revenue measurement would push the factor
higher still — a positive feedback loop that could spiral. Only token/request
volume and upstream cost drive the factor. This is a hard design constraint,
not a tunable.

**Cost uses configured channel costs, never billed amounts.** Upstream APIs
usually return token usage, not the final dollar amount charged to the
operator. Cost is therefore derived from the admin-configured
`channel_costs` map (a conservative USD upper bound per 1M total tokens per
channel) in `dynamic_pricing_setting`, multiplied by token volume. Billed
quota amounts are never used, because they already include the multiplier
itself (billing is downstream of pricing) and would make the cost signal
self-referential. Channels without a configured cost are excluded from the
window calculation and route selection; with `require_channel_cost=true`, a
request routed to one of them is rejected before upstream spend.

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
| `dynamic_pricing_setting.min_factor` | float | `1.0` | Operator-controlled minimum request multiplier while enabled. Must be finite, at least `1`, and no greater than `max_factor`. |
| `dynamic_pricing_setting.require_channel_cost` | bool | `true` | Fail closed before upstream spend when the selected channel has no positive configured conservative cost. It must remain `true` while the feature is enabled. |
| `dynamic_pricing_setting.tick_interval_seconds` | int | `60` | Sleep between ticks. Re-read every iteration; it must be positive. An invalid configuration sleeps at the 60 s fallback but skips the tick. |
| `dynamic_pricing_setting.window_minutes` | int | `5` | Sliding window length; consume logs in `[now−window, now]` are aggregated per tick. Must be positive or the tick is skipped. |
| `dynamic_pricing_setting.target_tpm` | float | `100000` | Tokens-per-minute target; one load dimension. |
| `dynamic_pricing_setting.target_rpm` | float | `60` | Requests-per-minute target; one load dimension. |
| `dynamic_pricing_setting.target_cost_rate` | float | `1.0` | Upstream-cost rate target, USD per minute; one load dimension. |
| `dynamic_pricing_setting.base_price_usd_per_million` | float | `1.0` | Lowest effective reference selling price in USD per 1M tokens used to convert upstream cost into a multiplier. Must be positive while enabled; overstating it weakens protection. |
| `dynamic_pricing_setting.alpha_load` | float | `0.3` | EMA smoothing for raw load (and for the measured unit cost). First sample is taken directly (cold start). |
| `dynamic_pricing_setting.alpha_up` | float | `0.30` | Asymmetric-EMA alpha used when the factor must rise. |
| `dynamic_pricing_setting.alpha_down` | float | `0.05` | Asymmetric-EMA alpha used when the factor must fall. |
| `dynamic_pricing_setting.cost_floor_factor` | float | `1.2` | Multiplies the effective upstream cost before it is compared with the model base price. Must be finite and at least `1`. |
| `dynamic_pricing_setting.max_factor` | float | `3.0` | Ceiling for the load-driven premium. `min_factor` cannot exceed it, but a known-cost hard floor may. |
| `dynamic_pricing_setting.load_deadzone` | float | `0.4` | Load at or below this fraction of target produces no heat. It must be finite and in `[0,1)`. |
| `dynamic_pricing_setting.heat_gamma` | float | `2.0` | Non-linear shaping exponent on heat; it must be finite and at least `1`. |
| `dynamic_pricing_setting.max_step_up` | float | `0.10` | Per-tick upward clamp: `mult ≤ prev × (1 + 0.10)`. |
| `dynamic_pricing_setting.max_step_down` | float | `0.03` | Per-tick downward clamp: `mult ≥ prev × (1 − 0.03)`. |
| `dynamic_pricing_setting.failover_probability` | float | `0.15` | Expected probability of failing over to the backup channel, used in the route cost; it must be finite and in `[0,1]`. No effect when the backup cost is unknown or ≤ cheap. |
| `dynamic_pricing_setting.channel_costs` | map (JSON) | `{}` | Conservative upstream cost upper bound in USD per 1M total tokens keyed by channel ID (string keys, e.g. `{"12":1.8}`). Values must be positive; every active channel is required before enabling through the settings endpoint. |
| `dynamic_pricing_setting.per_model` | map (JSON) | `{}` | Per-model overrides of the three load targets and optional `base_price_usd_per_million`, e.g. `{"gpt-5":{"target_tpm":50000,"base_price_usd_per_million":8}}`. A zero override field (or an absent model) inherits the global value. |

### How keys are applied

The admin console exposes the feature at **System Settings → Models → Dynamic
Group Multiplier**. Its switch, minimum, safety inputs, active-channel cost
table, and per-model live preview use the dedicated root-auth endpoint
`PUT /api/dynamic_pricing/setting`. The update is validated as one
configuration, persisted in one DB transaction, and applies the master switch
last when enabling. A successful save triggers an immediate engine tick.

Keys may also be written through the standard admin options API
(`PUT /api/option`, root auth) as `dynamic_pricing_setting.<field>` and stored
in `OptionMap`. The update path then routes every `*.*` key through
`handleConfigUpdate`, which splits on the first `.`, resolves the config
module name, and applies the field via `config.UpdateConfigFromMap` (matching
each field's JSON tag). Because `dynamic_pricing_setting` is a registered
config module, **all of its keys flow through automatically** — no code change
or restart is required. The ticker re-reads the setting on every iteration, so
changes take effect on the next tick. Map fields (`channel_costs`,
`per_model`) are set as JSON strings.

Update safety guarantees:

- Single-key and bulk option writes strictly parse and validate the resulting
  full configuration before writing. Cross-field updates (for example costs +
  enable) are validated together.
- The request path re-evaluates the selected channel on every retry. A costlier
  retry raises (never lowers) the captured multiplier and reserves the larger
  pre-consume amount before the next upstream attempt.
- A zero/negative effective model or group billing base is rejected while the
  feature is enabled: multiplying a free price can never recover upstream
  cost.
- Invalid configuration or unknown selected-channel cost fails closed.

## Status API

`GET /api/dynamic_pricing/status` (admin auth) returns a read-only snapshot:

```json
{
  "success": true,
  "message": "",
  "data": {
    "enabled": true,
    "preview_factor": 2.16,
    "setting": {
      "enabled": true,
      "min_factor": 1.2,
      "require_channel_cost": true,
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
      "channel_costs": {"12": 1.8},
      "per_model": {}
    },
    "models": {
      "gpt-5": {
        "factor": 2.16,
        "request_factor_min": 2.16,
        "request_factor_max": 2.16,
        "engine_factor": 2.16,
        "hard_cost_floor": 2.16,
        "load_ema": 0.7,
        "cost_ema": 1.8,
        "has_unpriced_traffic": false,
        "unpriced_tokens": 0,
        "unpriced_requests": 0,
        "updated_at": 1786000000
      }
    },
    "safety": {
      "ready": true,
      "status": "ready",
      "active_channel_count": 1,
      "configured_channel_count": 1,
      "missing_channels": []
    }
  }
}
```

- `setting` is the full `DynamicPricingSetting` (no sensitive fields).
- `models` contains only models with current in-memory state (models that were
  ticked at least once since start, or whose state was loaded from Redis).
  Each entry exposes the engine factor, the immediate request-factor range
  across configured active channels, the latest observed hard cost floor,
  EMAs, unknown-cost counters, and last tick time.
- `safety` reports active-channel cost coverage for the UI. The UI polls this
  endpoint every three seconds.

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
channels. If a window contains only unpriced traffic, the engine falls back
to `min_factor`; strict request pricing still blocks the unknown-cost channel.

**Zero-traffic models decay.** The ticker iterates models already held in
memory even when the current window has no consume rows. Each such model gets
a zero-load sample, so its factor and EMAs move down under the configured
step/EMA controls. Redis restoration preserves a known hard cost floor even
when it exceeds `max_factor`.

**Load above target only warns.** When `load_ema > 1.0` the ticker logs a
warning; load alone never rejects or sheds traffic. Unknown-cost protection is
a separate fail-closed billing safeguard.

## Known limitations / out of scope

- **Not capacity control.** Exceeding the load targets only raises the price
  factor (and logs a warning). Dynamic pricing cannot throttle, queue, or
  reject requests — it is a pricing signal, and cannot replace capacity
  control.
- **Configured costs are operator inputs.** Token usage can come from the
  upstream response (or an estimate when usage is absent), but the dollar
  tariff does not. Cost-map accuracy and a conservative reference selling
  price remain operator responsibilities.
- **Pricing integration.** The multiplier is captured as an extra
  `dynamic_pricing` ratio for fixed-price, ratio, per-call, and tiered
  pre-consume paths. Audio/realtime settlement and tiered post-settlement use
  the same captured ratio. Midjourney per-call quota and task models listed in
  `TASK_PRICE_PATCH` still apply this safety ratio. Task submit-time ratio
  replacement merges the captured dynamic ratio instead of dropping it.
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

- These are steady-state values. A load-only jump from `1.0×` toward `3.0×`
  climbs roughly `+10%` per tick with the defaults, while relief falls at
  most `3%` per tick (and the `alpha_down = 0.05` EMA slows it further). A
  known-cost floor is applied immediately and does not wait for either clamp.
- Earlier planning notes quoted `~1.65×` at 70% and `~2.45×` at 90%; those
  figures assume a slightly lighter shaping (e.g. `load_deadzone ≈ 0.3` or
  `heat_gamma ≈ 1.7`). The exact multiplier is fully determined by the
  formulas in [Formula pipeline](#formula-pipeline) with whatever parameters
  are configured — there is no other shaping.
- The multiplier applies on top of the configured price: a model priced at
  `$10 / 1M` tokens charges `$10 × factor` (rounded to quota units) while the
  feature is active for it.
