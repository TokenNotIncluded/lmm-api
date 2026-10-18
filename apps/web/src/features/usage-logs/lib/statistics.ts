/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'
import { historicalQuotaUSD } from '@/lib/historical-quota-money'
import { useAuthStore } from '@/stores/auth-store'

import type { GetLogStatsParams, LogStatistics } from '../types'
import { buildQueryParams } from './query-params'

const count = z.number().int().safe().nonnegative()
const statisticsResponseSchema = z.object({
  success: z.literal(true),
  data: z.object({
    quota: count,
    rpm: count,
    tpm: count,
    quota_usd: z.string().optional(),
    quota_money_available: z.boolean().optional(),
    quota_money_basis: z.string().optional(),
    unpriced_quota_records: count.optional(),
    quota_projection_truncated: z.boolean().optional(),
  }),
})

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

export function parseUsageStatistics(value: unknown): LogStatistics {
  const result = statisticsResponseSchema.safeParse(value)
  if (!result.success) {
    throw new Error('Usage statistics unavailable')
  }
  const statistics = result.data.data
  const projectedUSD = historicalQuotaUSD(statistics)
  if (
    projectedUSD !== null &&
    ((statistics.quota === 0) !== (projectedUSD === 0) ||
      (statistics.quota_money_basis === 'empty_result' &&
        statistics.quota !== 0))
  ) {
    // Keep proven counters, but never let contradictory money hide their raw value.
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
): Promise<LogStatistics> {
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
