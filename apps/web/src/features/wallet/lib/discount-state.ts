/*
Copyright (C) 2026 LIghtJUNction
*/

export interface AppliedDiscountState {
  code: string
  percent: number | null
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
