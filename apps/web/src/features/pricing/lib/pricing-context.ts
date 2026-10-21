/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { PricingModel } from '../types'

function getPricingBasis(model: PricingModel) {
  if (model.quota_type !== 2) return model.pricing_context?.rate_basis
  const firstVariant = model.image_variants?.[0]
  return firstVariant?.rate_basis &&
    model.image_variants?.every(
      (variant) => variant.rate_basis === firstVariant.rate_basis
    )
    ? firstVariant.rate_basis
    : undefined
}

export function hasAccountPricingContext(model: PricingModel): boolean {
  return (
    getPricingBasis(model) === 'effective_user_rate' &&
    model.pricing_context?.user_price?.status === 'verified'
  )
}

export function hasCurrentPricingContext(model: PricingModel): boolean {
  return (
    getPricingBasis(model) === 'base_group_rate' ||
    getPricingBasis(model) === 'effective_user_rate' ||
    (model.quota_type === 2 &&
      (model.pricing_context?.rate_basis === 'base_group_rate' ||
        model.pricing_context?.rate_basis === 'effective_user_rate' ||
        Boolean(model.image_variants?.some((variant) => variant.rate_basis))))
  )
}

export function getPricingBasisLabel(
  model: PricingModel,
  legacyLabel = 'Base price (1×)'
): string {
  if (!hasCurrentPricingContext(model)) return legacyLabel
  if (model.quota_type === 2 && getPricingBasis(model) === undefined) {
    return model.image_variants?.length ? 'Image prices' : 'Price'
  }
  return hasAccountPricingContext(model)
    ? 'Current account price'
    : 'Group base price'
}

/** Display the exact account factor; this value must never calculate a rate. */
export function getAccountPriceMultiplier(model: PricingModel): string | null {
  if (!hasAccountPricingContext(model)) return null
  const userPrice = model.pricing_context?.user_price
  if (userPrice?.status !== 'verified') return null
  const { numerator, denominator } = userPrice.multiplier
  if (!/^[1-9]\d*$/.test(numerator) || !/^[1-9]\d*$/.test(denominator)) {
    return null
  }
  const originalNumerator = BigInt(numerator)
  const originalDenominator = BigInt(denominator)
  if (originalNumerator > originalDenominator) return null
  let scaled = originalNumerator * 100n
  let divisor = originalDenominator
  let a = scaled
  let b = divisor
  while (b !== 0n) [a, b] = [b, a % b]
  scaled /= a
  divisor /= a
  let remaining = divisor
  for (const factor of [2n, 5n]) {
    while (remaining % factor === 0n) remaining /= factor
  }
  if (remaining !== 1n) return `${numerator}/${denominator}×`
  const whole = scaled / divisor
  let remainder = scaled % divisor
  let decimals = ''
  while (remainder !== 0n) {
    remainder *= 10n
    decimals += String(remainder / divisor)
    remainder %= divisor
  }
  return `${whole}${decimals ? `.${decimals}` : ''}%`
}
