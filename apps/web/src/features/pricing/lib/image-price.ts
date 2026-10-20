/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import type { PricingImageVariant } from '../types'

/** Preserve exact nonterminating ratios instead of rounding a small price to zero. */
function exactFraction(numerator: bigint, denominator: bigint): string {
  const whole = numerator / denominator
  let remainder = numerator % denominator
  let decimals = ''
  for (let index = 0; remainder !== 0n && index < 18; index++) {
    remainder *= 10n
    decimals += (remainder / denominator).toString()
    remainder %= denominator
  }
  return remainder === 0n
    ? `${whole}${decimals ? `.${decimals}` : ''}`
    : `${numerator}/${denominator}`
}

export function formatImageVariantUSD(variant: PricingImageVariant): string {
  const price = variant.usd_micros_per_image
  return `${exactFraction(BigInt(price.numerator), BigInt(price.denominator) * 1_000_000n)} USD`
}

export function formatImageVariantCredits(
  variant: PricingImageVariant
): string {
  const price = variant.credits_per_image
  return `${exactFraction(BigInt(price.numerator), BigInt(price.denominator))} Credits`
}
