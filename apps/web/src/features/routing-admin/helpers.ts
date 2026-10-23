/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import type { Price, PriorityChange, Rate, Target } from './contract'

/** Exact amounts are never parsed through JavaScript Number. */
export function formatRate(
  rate: Rate['amount'],
  group = 1000000,
  speed = 1000000
): string {
  let n = BigInt(rate.numerator) * BigInt(group) * BigInt(speed),
    d = BigInt(rate.denominator) * 1000000000000n
  let a = n,
    b = d
  while (b !== 0n) {
    ;[a, b] = [b, a % b]
  }
  n /= a
  d /= a
  const whole = n / d
  let remainder = n % d,
    decimal = ''
  for (let i = 0; remainder !== 0n && i < 18; i++) {
    remainder *= 10n
    decimal += (remainder / d).toString()
    remainder %= d
  }
  return remainder === 0n
    ? `${whole}${decimal ? `.${decimal}` : ''} USD`
    : `${n}/${d} USD`
}
export function effectiveRate(price: Price, rate: Rate) {
  return formatRate(
    rate.amount,
    price.group_multiplier_ppm,
    price.speed_multiplier_ppm
  )
}
export function priorityChanges(
  before: readonly Target[],
  after: readonly Target[]
): PriorityChange[] {
  if (
    before.length !== after.length ||
    new Set(after.map((t) => t.target_id)).size !== after.length
  )
    throw Error('Invalid targets')
  return after.flatMap((target) => {
    const original = before.find((t) => t.target_id === target.target_id)
    if (!original) throw Error('Invalid target')
    return target.priority === original.priority
      ? []
      : [{ target_id: target.target_id, priority: target.priority }]
  })
}
export function applyPriorities(
  targets: readonly Target[],
  changes: readonly PriorityChange[]
): Target[] {
  const map = new Map(changes.map((c) => [c.target_id, c.priority]))
  return targets.map((t) => ({
    ...t,
    priority: map.get(t.target_id) ?? t.priority,
  }))
}
/** Smaller numbers win; equal numbers preserve weighted competition. */
export function movePriority(
  targets: readonly Target[],
  movedId: string,
  anchorId: string,
  position: 'before' | 'same' | 'after'
): PriorityChange[] {
  if (targets.length > 64 || movedId === anchorId) return []
  const moved = targets.find((t) => t.target_id === movedId),
    anchor = targets.find((t) => t.target_id === anchorId)
  if (!moved || !anchor) throw Error('Invalid target')
  if (position === 'same')
    return moved.priority === anchor.priority
      ? []
      : [{ target_id: movedId, priority: anchor.priority }]
  const remaining = targets.filter((t) => t.target_id !== movedId)
  const tiers = [...new Set(remaining.map((t) => t.priority))].sort(
    (a, b) => a - b
  )
  const index = tiers.indexOf(anchor.priority) + (position === 'after' ? 1 : 0)
  const lower = tiers[index - 1] ?? -1,
    upper = tiers[index] ?? 65536
  if (upper - lower > 1) {
    const priority = Math.floor((lower + upper) / 2)
    return priority === moved.priority ? [] : [{ target_id: movedId, priority }]
  }
  // No integer gap: re-space the entire affected route, keeping every existing
  // tier together. At most 64 targets fit comfortably in the u16 range.
  const after = targets.map((t) => ({
    ...t,
    priority:
      t.target_id === movedId
        ? index * 10
        : (tiers.indexOf(t.priority) +
            (tiers.indexOf(t.priority) >= index ? 1 : 0)) *
          10,
  }))
  return priorityChanges(targets, after)
}
export const adjustmentLabels = {
  keep: 'Keep source price',
  multiply: 'Multiply by site exchange rate',
  divide: 'Divide by site exchange rate',
} as const
export const rateLabels = {
  input: 'Input',
  output: 'Output',
  cache_read: 'Cache Read',
  cache_write: 'Cache Write',
  cache_write_1h: 'Cache write (1 hour)',
  cache_write_12h: 'Cache write (12 hours)',
  request: 'Request',
  search: 'Search',
  x_posts_fetched: 'X posts fetched',
  x_users_fetched: 'X users fetched',
  embedding_text: 'Text tokens',
  embedding_image: 'Image tokens',
} as const
export const unitLabels = {
  million_tokens: 'per million tokens',
  request: 'per request',
  search_call: 'per search call',
  search_query: 'per search query',
  grounded_request: 'per grounded request',
  item: 'per item',
} as const
