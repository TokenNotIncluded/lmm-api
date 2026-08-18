/*
Copyright (C) 2026 LIghtJUNction
*/

export interface AppliedDiscountState {
  code: string
  percent: number | null
}

/**
 * Calculates the amount removed by an administrator discount code from the
 * final server quote. Discount codes are applied after the platform quote, so
 * reversing the code multiplier gives a stable client-side explanation while
 * the server remains authoritative for checkout.
 */
export function discountCodeSavings(
  finalAmount: number,
  discountPercent: number | null | undefined
): number {
  if (
    !Number.isFinite(finalAmount) ||
    finalAmount <= 0 ||
    !Number.isFinite(discountPercent) ||
    discountPercent === undefined ||
    discountPercent === null ||
    discountPercent <= 0 ||
    discountPercent >= 100
  ) {
    return 0
  }

  const multiplier = (100 - discountPercent) / 100
  return Math.max(0, finalAmount / multiplier - finalAmount)
}

/**
 * A discount validation is tied to the credited amount. Keep the user's
 * entered code visible, but discard the server-validated state when that
 * amount changes so the checkout cannot advertise a stale discount.
 */
export function discountAfterAmountChange(
  state: AppliedDiscountState,
  previousAmount: number,
  nextAmount: number
): AppliedDiscountState {
  if (previousAmount === nextAmount || !state.code.trim()) {
    return state
  }

  return { code: '', percent: null }
}
