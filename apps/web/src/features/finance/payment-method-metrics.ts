/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import type { FinanceMethodMetric } from './api'

export type PaymentMethodSummary = {
  revenueMicros: number
  refundMicros: number
  netRevenueMicros: number
}

function amountForMethod(
  metrics: FinanceMethodMetric[] | undefined,
  method: string
): number {
  return (metrics ?? []).reduce(
    (total, metric) =>
      metric.method === method ? total + metric.amount_micros : total,
    0
  )
}

/**
 * A configured payment method can have more than one provider-backed metric.
 * Keep the compact finance row reconciled to the overview totals by summing
 * every metric for that method before subtracting refunds.
 */
export function paymentMethodSummary(
  method: string,
  revenue: FinanceMethodMetric[] | undefined,
  refunds: FinanceMethodMetric[] | undefined
): PaymentMethodSummary {
  const revenueMicros = amountForMethod(revenue, method)
  const refundMicros = amountForMethod(refunds, method)
  return {
    revenueMicros,
    refundMicros,
    netRevenueMicros: revenueMicros - refundMicros,
  }
}
