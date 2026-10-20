/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'
import { historicalQuotaUSD } from '@/lib/historical-quota-money'
import { useAuthStore } from '@/stores/auth-store'

import type { GetLogStatsParams } from '../types'
import { buildQueryParams } from './query-params'

const count = z.number().int().safe().nonnegative()
const nativeStatisticsSchema = z.object({
  quota: count,
  rpm: count,
  tpm: count,
  quota_usd: z.string().optional(),
  quota_money_available: z.boolean().optional(),
  quota_money_basis: z.string().optional(),
  gross_settled_credits: count.optional(),
  refunded_credits: count.optional(),
  net_consumption_credits: count.optional(),
  billing_basis: z.literal('net_consumption').optional(),
  wallet_debit_supported: z.literal(false).optional(),
  unpriced_quota_records: count.optional(),
  quota_projection_truncated: z.boolean().optional(),
  source: z.never().optional(),
  raw_consumption_quota: z.never().optional(),
  quota_numeric_representable: z.never().optional(),
  invalid_quota_records: z.never().optional(),
  rates_available: z.literal(true).optional(),
  reserved_credits: count.optional(),
  incomplete_count: count.optional(),
  output_image_count: count.optional(),
  token_count_scope: z.literal('settled_text_requests').optional(),
  platform_sponsored_quota: count.optional(),
  personal_wallet_quota: count.optional(),
  personal_wallet_quota_basis: z
    .literal('wallet_source_net_consumption_not_debit')
    .optional(),
  team_funded_quota: count.optional(),
  settlement_scope: z.literal('settled_requests').optional(),
  spending_scope: z.literal('all_payers').optional(),
  usage_scope: z
    .enum(['actor_personal_key', 'actor_personal_usage'])
    .optional(),
})
const historicalStatisticsFields = {
  source: z.literal('legacy_log_only'),
  raw_consumption_quota: z.string(),
  invalid_quota_records: count,
  unpriced_quota_records: count,
  quota_projection_truncated: z.boolean().optional(),
  quota_usd: z.string().nullable(),
  quota_money_available: z.boolean(),
  quota_money_basis: z.string(),
  rates_available: z.literal(false),
  rpm: z.null(),
  tpm: z.null(),
}
const historicalStatisticsSchema = z.discriminatedUnion(
  'quota_numeric_representable',
  [
    z.object({
      ...historicalStatisticsFields,
      quota_numeric_representable: z.literal(true),
      quota: count,
    }),
    z.object({
      ...historicalStatisticsFields,
      quota_numeric_representable: z.literal(false),
      quota: z.null(),
    }),
  ]
)
const statisticsDataSchema = z.union([
  nativeStatisticsSchema,
  historicalStatisticsSchema,
])
const statisticsResponseSchema = z.object({
  success: z.literal(true),
  data: statisticsDataSchema,
})

export type ParsedUsageStatistics = z.infer<typeof statisticsDataSchema>

export type UsageStatisticsScope = {
  userId: number
  sessionId: string | undefined
  role: number
}

export function readUsageStatisticsScope(
  auth = useAuthStore.getState().auth
): UsageStatisticsScope | undefined {
  const { user, session } = auth
  if (
    !user ||
    !Number.isSafeInteger(user.id) ||
    user.id <= 0 ||
    !Number.isSafeInteger(user.role)
  ) {
    return undefined
  }
  return { userId: user.id, sessionId: session?.sid, role: user.role }
}

function assertScope(scope: UsageStatisticsScope, signal?: AbortSignal) {
  const current = readUsageStatisticsScope()
  if (
    signal?.aborted ||
    current?.userId !== scope.userId ||
    current?.sessionId !== scope.sessionId ||
    current?.role !== scope.role
  ) {
    throw new Error('Usage statistics session changed')
  }
}

export function parseUsageStatistics(value: unknown): ParsedUsageStatistics {
  const result = statisticsResponseSchema.safeParse(value)
  if (!result.success) {
    throw new Error('Usage statistics unavailable')
  }
  const statistics = result.data.data
  const projectedUSD = historicalQuotaUSD({
    ...statistics,
    quota_usd: statistics.quota_usd ?? undefined,
  })
  if (
    projectedUSD !== null &&
    (statistics.quota === null ||
      (statistics.invalid_quota_records ?? 0) > 0 ||
      (statistics.quota === 0) !== (projectedUSD === 0) ||
      (statistics.quota_money_basis === 'empty_result' &&
        statistics.quota !== 0))
  ) {
    // Keep proven counters, but never let contradictory money hide their raw value.
    if (statistics.source === 'legacy_log_only') {
      return {
        ...statistics,
        quota_usd: null,
        quota_money_available: false,
        quota_money_basis: 'inconsistent_statistics_projection',
      }
    }
    const { quota_usd: _untrustedUSD, ...counters } = statistics
    return {
      ...counters,
      quota_money_available: false,
      quota_money_basis: 'inconsistent_statistics_projection',
    }
  }
  return statistics
}

export async function readUsageStatistics(
  scope: UsageStatisticsScope,
  params: GetLogStatsParams,
  isAdmin: boolean,
  signal?: AbortSignal
): Promise<ParsedUsageStatistics> {
  assertScope(scope, signal)
  const endpoint = isAdmin ? '/api/log/stat' : '/api/log/self/stat'
  const query = buildQueryParams({ ...params })
  const response = await api.get<unknown>(`${endpoint}?${query}`, {
    authScope: { userId: scope.userId, sessionId: scope.sessionId },
    signal,
    disableDuplicate: true,
    skipBusinessError: true,
    skipErrorHandler: true,
    skipAuthRefresh: true,
    timeout: 10_000,
  })
  assertScope(scope, signal)
  return parseUsageStatistics(response.data)
}
