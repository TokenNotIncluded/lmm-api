/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativeUpstreamCostPrice } from '../catalog-contract'
import type { PricingModel } from '../types'
import { getDisplayPriceGroup } from './model-helpers'

export function getUpstreamCostPrice(
  model: PricingModel,
  selectedGroup?: string
) {
  const group = getDisplayPriceGroup(model, selectedGroup)
  const price = group ? model.group_upstream_cost_prices?.[group] : undefined
  return group && price
    ? { group, price, reason: model.group_unavailable_reasons?.[group] }
    : null
}

/** Display the server's effective factor; never apply account discounts again. */
export function formatUpstreamMultiplier(
  price: NativeUpstreamCostPrice
): string {
  let n = BigInt(price.multiplier.numerator)
  let d = BigInt(price.multiplier.denominator)
  let a = n
  let b = d
  while (b !== 0n) [a, b] = [b, a % b]
  n /= a
  d /= a
  return d === 1n ? String(n) : `${n}/${d}`
}
