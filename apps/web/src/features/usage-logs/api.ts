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
import { isAxiosError } from 'axios'

import { api } from '@/lib/api'

import {
  parseHistoryUnavailable,
  parseLogsPage,
  type LogsPage,
} from './lib/history-response'
import { buildQueryParams } from './lib/query-params'
import {
  readUsageStatisticsScope,
  type UsageStatisticsScope,
} from './lib/statistics'
import type {
  GetLogsParams,
  GetLogsResponse,
  GetLogStatsParams,
  GetLogStatsResponse,
  GetMidjourneyLogsParams,
  GetTaskLogsParams,
  UserInfo,
  LogCategory,
} from './types'

// ============================================================================
// Generic API Helpers
// ============================================================================

function buildApiPath(endpoint: string, isAdmin: boolean): string {
  return isAdmin ? endpoint : `${endpoint}/self`
}

function assertLogScope(scope: UsageStatisticsScope, signal?: AbortSignal) {
  const current = readUsageStatisticsScope()
  if (
    signal?.aborted ||
    current?.userId !== scope.userId ||
    current?.sessionId !== scope.sessionId ||
    current?.role !== scope.role
  ) {
    throw new Error('Usage history session changed')
  }
}

export async function readLogsPage({
  category,
  params,
  isAdmin,
  signal,
  scope: expectedScope,
}: {
  category: LogCategory
  params: GetLogsParams | GetMidjourneyLogsParams | GetTaskLogsParams
  isAdmin: boolean
  signal?: AbortSignal
  scope?: UsageStatisticsScope
}): Promise<LogsPage> {
  const scope = expectedScope ?? readUsageStatisticsScope()
  if (!scope) throw new Error('Usage history session unavailable')
  assertLogScope(scope, signal)
  const endpoint =
    category === 'common'
      ? '/api/log'
      : category === 'drawing'
        ? '/api/mj'
        : '/api/task'
  const query = buildQueryParams({ p: 1, page_size: 20, ...params })
  let payload: unknown
  try {
    const response = await api.get<unknown>(
      `${buildApiPath(endpoint, isAdmin)}?${query}`,
      {
        signal,
        authScope: { userId: scope.userId, sessionId: scope.sessionId },
        disableDuplicate: true,
        skipBusinessError: true,
        skipErrorHandler: true,
        skipAuthRefresh: true,
        timeout: 10_000,
      }
    )
    payload = response.data
  } catch (error) {
    assertLogScope(scope, signal)
    if (isAxiosError(error) && error.response?.status === 503) {
      const unavailable = parseHistoryUnavailable(error.response.data, category)
      if (unavailable) throw unavailable
    }
    throw error
  }
  assertLogScope(scope, signal)
  const unavailable = parseHistoryUnavailable(payload, category)
  if (unavailable) throw unavailable
  return parseLogsPage(payload, category)
}

async function fetchLogs<
  T extends GetLogsParams | GetMidjourneyLogsParams | GetTaskLogsParams,
>(
  category: LogCategory,
  params: T,
  isAdmin: boolean
): Promise<GetLogsResponse> {
  return {
    success: true,
    data: await readLogsPage({ category, params, isAdmin }),
  }
}

async function fetchLogStats<T>(
  endpoint: string,
  params: T,
  isAdmin: boolean
): Promise<GetLogStatsResponse> {
  const queryParams = buildQueryParams(
    params as unknown as Record<string, unknown>
  )
  const path = buildApiPath(endpoint, isAdmin)
  const res = await api.get(`${path}/stat?${queryParams}`)
  return res.data
}

// ============================================================================
// Common Log APIs
// ============================================================================

export const getAllLogs = (params: GetLogsParams = {}) =>
  fetchLogs('common', params, true)

export const getUserLogs = (
  params: Omit<GetLogsParams, 'username' | 'channel'> = {}
) => fetchLogs('common', params, false)

export const getLogStats = (params: GetLogStatsParams = {}) =>
  fetchLogStats('/api/log', params, true)

export const getUserLogStats = (
  params: Omit<GetLogStatsParams, 'username' | 'channel'> = {}
) => fetchLogStats('/api/log', params, false)

export async function getUserInfo(
  userId: number
): Promise<{ success: boolean; message?: string; data?: UserInfo }> {
  const res = await api.get(`/api/user/${userId}`)
  return res.data
}

// ============================================================================
// MjProxy (Drawing) Logs API
// ============================================================================

export const getAllMidjourneyLogs = (params: GetMidjourneyLogsParams) =>
  fetchLogs('drawing', params, true)

export const getUserMidjourneyLogs = (params: GetMidjourneyLogsParams) =>
  fetchLogs('drawing', params, false)

// ============================================================================
// Task Logs API
// ============================================================================

export const getAllTaskLogs = (params: GetTaskLogsParams) =>
  fetchLogs('task', params, true)

export const getUserTaskLogs = (params: GetTaskLogsParams) =>
  fetchLogs('task', params, false)
