/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
export const ROUTER_MODELS = [
  'nvidia/switchyard',
  'openrouter/auto-beta',
  'openrouter/fusion',
  'typesafe/jev-router',
] as const

/** Owner-file preview only. It cannot authorize a route or change runtime prices. */
export function routerPriceDraft(
  id: string,
  model: string,
  group: string,
  multiplier: string,
  groups: string[]
) {
  if (
    !/^[1-9]\d{0,15}$/.test(id) ||
    !Number.isSafeInteger(Number(id)) ||
    !ROUTER_MODELS.some((value) => value === model) ||
    !groups.includes(group) ||
    !/^(0|[1-9]\d{0,3})(\.\d{1,6})?$/.test(multiplier)
  ) {
    return null
  }
  const [whole = '0', fraction = ''] = multiplier.split('.')
  const ppm = BigInt(whole) * 1_000_000n + BigInt(fraction.padEnd(6, '0'))
  if (ppm <= 0n || ppm > 1_000_000_000n) return null
  return JSON.stringify(
    {
      id: Number(id),
      model,
      group,
      speed: 'standard',
      billing_mode: 'upstream_cost',
      rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
      group_multiplier_ppm: Number(ppm),
      speed_multiplier_ppm: 1_000_000,
    },
    null,
    2
  )
}
