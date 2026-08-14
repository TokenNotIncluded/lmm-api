/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
/*
Copyright (C) 2026 LIghtJUNction
*/
import { api } from '@/lib/api'

export interface FinanceMethodMetric {
  method: string
  provider: string
  category?: string
  amount_micros: number
  orders: number
  users: number
  token_units: number
}

export interface FinanceDailyMetric {
  date: string
  revenue_micros: number
  expense_micros: number
  profit_micros: number
  token_units: number
  requests: number
}

export interface FinanceUserMetric {
  user_id: number
  revenue_micros: number
  expense_micros: number
  token_cost_micros: number
  token_units: number
  requests: number
}

export interface FinancePaymentMethod {
  id?: number
  method: string
  label: string
  enabled: boolean
  include_revenue: boolean
}

export interface FinanceOverview {
  range: { start: number; end: number }
  currency: string
  revenue_micros: number
  expense_micros: number
  profit_micros: number
  revenue_by_method: FinanceMethodMetric[]
  expense_by_method: FinanceMethodMetric[]
  tokens: {
    prompt_tokens: number
    completion_tokens: number
    total_tokens: number
    requests: number
    estimated_cost_micros: number
    unpriced_requests: number
  }
  daily: FinanceDailyMetric[]
  users: FinanceUserMetric[]
  payment_methods: FinancePaymentMethod[]
  sources_bounded: boolean
}

export interface FinanceLedgerEntry {
  id: number
  entry_type: string
  category: string
  amount_micros: number
  currency: string
  direction: number
  payment_method: string
  payment_provider: string
  user_id?: number
  source_type: string
  source_id: string
  note: string
  occurred_at: number
  created_at: number
  created_by: number
  reversal_of_id?: number
}

interface FinanceEnvelope<T> {
  success: boolean
  data: T
  message?: string
}

function rangeParams(days: number, paymentMethod?: string) {
  const end = Math.floor(Date.now() / 1000)
  const params: Record<string, number | string> = {
    start_timestamp: end - days * 24 * 60 * 60,
    end_timestamp: end,
  }
  if (paymentMethod) params.payment_method = paymentMethod
  return params
}

export async function getFinanceOverview(days = 30, paymentMethod?: string) {
  const response = await api.get<FinanceEnvelope<FinanceOverview>>(
    '/api/finance/overview',
    { params: rangeParams(days, paymentMethod) }
  )
  return response.data
}

export async function getFinanceUser(userId: number, days = 30) {
  const response = await api.get<FinanceEnvelope<FinanceOverview>>(
    `/api/finance/users/${userId}`,
    { params: rangeParams(days) }
  )
  return response.data
}

export async function createFinanceExpense(input: {
  category: string
  amount_micros: number
  currency: string
  note?: string
  occurred_at?: number
  idempotency_key?: string
}) {
  const response = await api.post<
    FinanceEnvelope<{ entry: FinanceLedgerEntry }>
  >('/api/finance/entries', { entry_type: 'expense', ...input })
  return response.data
}

export async function updateFinancePaymentMethod(
  method: string,
  input: Partial<
    Pick<FinancePaymentMethod, 'label' | 'enabled' | 'include_revenue'>
  >
) {
  const response = await api.put<FinanceEnvelope<FinancePaymentMethod>>(
    `/api/finance/payment-methods/${encodeURIComponent(method)}`,
    input
  )
  return response.data
}
