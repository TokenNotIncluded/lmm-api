/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'

import { api } from '@/lib/api'

import type { GetLogsParams } from '../types'
import { buildQueryParams } from './query-params'
import {
  readUsageStatisticsScope,
  type UsageStatisticsScope,
} from './statistics'

export const MAX_USAGE_CSV_BYTES = 32 * 1024 * 1024
const MAX_ROWS = 10000
const MAX_SECONDS = 31 * 86400

export type CsvExportFailure =
  | 'invalid'
  | 'too_dense'
  | 'busy'
  | 'timeout'
  | 'forbidden'
  | 'unavailable'
  | 'session_changed'
  | 'cancelled'
  | 'incomplete'

export class UsageCsvExportError extends Error {
  constructor(readonly reason: CsvExportFailure) {
    super(`Usage CSV export: ${reason}`)
  }
}

/** Export uses the applied request filters, never the current table page. */
export function usageCsvParams(params: GetLogsParams): URLSearchParams {
  const { p: _page, page_size: _size, ...filters } = params
  const start = filters.start_timestamp
  const end = filters.end_timestamp
  if (
    !Number.isSafeInteger(start) ||
    !Number.isSafeInteger(end) ||
    start === undefined ||
    end === undefined ||
    start < 0 ||
    end < start ||
    end > 253402300799 ||
    end - start > MAX_SECONDS
  )
    throw new UsageCsvExportError('invalid')
  return buildQueryParams(filters)
}

function assertScope(scope: UsageStatisticsScope, signal?: AbortSignal) {
  if (signal?.aborted) throw new UsageCsvExportError('cancelled')
  const current = readUsageStatisticsScope()
  if (
    current?.userId !== scope.userId ||
    current?.sessionId !== scope.sessionId ||
    current?.role !== scope.role
  )
    throw new UsageCsvExportError('session_changed')
}

export type UsageCsvDocument = {
  blob: Blob
  filename: 'lmm-personal-usage.csv' | 'lmm-historical-usage.csv'
  rows: number
}

function responseDocument(
  data: unknown,
  headers: Record<string, unknown>,
  isAdmin: boolean
): UsageCsvDocument {
  const source = isAdmin ? 'legacy_log_only' : 'native_personal_usage'
  const rows = String(headers['x-export-rows'] ?? '')
  const bytes = String(headers['x-export-bytes'] ?? '')
  if (
    !(data instanceof ArrayBuffer) ||
    data.byteLength > MAX_USAGE_CSV_BYTES ||
    !/^text\/csv(?:;|$)/i.test(String(headers['content-type'] ?? '')) ||
    headers['x-export-source'] !== source ||
    !/^(0|[1-9]\d{0,4})$/.test(rows) ||
    Number(rows) > MAX_ROWS ||
    !/^[1-9]\d{0,7}$/.test(bytes) ||
    Number(bytes) !== data.byteLength
  )
    throw new UsageCsvExportError('incomplete')
  const prefix = new Uint8Array(data, 0, Math.min(data.byteLength, 3))
  if (prefix[0] !== 0xef || prefix[1] !== 0xbb || prefix[2] !== 0xbf) {
    throw new UsageCsvExportError('incomplete')
  }
  return {
    blob: new Blob([data], { type: 'text/csv;charset=utf-8' }),
    filename: isAdmin ? 'lmm-historical-usage.csv' : 'lmm-personal-usage.csv',
    rows: Number(rows),
  }
}

/** One authenticated request; cancelled or stale sessions never yield a file. */
export async function readUsageCsv({
  params,
  isAdmin,
  scope,
  signal,
}: {
  params: GetLogsParams
  isAdmin: boolean
  scope: UsageStatisticsScope
  signal?: AbortSignal
}): Promise<UsageCsvDocument> {
  assertScope(scope, signal)
  const query = usageCsvParams(params)
  const controller = new AbortController()
  let oversized = false
  const cancel = () => controller.abort()
  signal?.addEventListener('abort', cancel, { once: true })
  try {
    const result = await api.get<unknown>(
      `${isAdmin ? '/api/log/export' : '/api/log/self/export'}?${query}`,
      {
        responseType: 'arraybuffer',
        signal: controller.signal,
        authScope: { userId: scope.userId, sessionId: scope.sessionId },
        disableDuplicate: true,
        skipBusinessError: true,
        skipErrorHandler: true,
        skipAuthRefresh: true,
        // The server separately bounds generation and transfer to 60 seconds.
        timeout: 125_000,
        maxContentLength: MAX_USAGE_CSV_BYTES,
        onDownloadProgress: ({ loaded, total }) => {
          if (
            loaded > MAX_USAGE_CSV_BYTES ||
            (total !== undefined && total > MAX_USAGE_CSV_BYTES)
          ) {
            oversized = true
            controller.abort()
          }
        },
      }
    )
    assertScope(scope, signal)
    if (oversized) throw new UsageCsvExportError('too_dense')
    return responseDocument(result.data, result.headers, isAdmin)
  } catch (error) {
    assertScope(scope, signal)
    if (oversized) throw new UsageCsvExportError('too_dense')
    if (error instanceof UsageCsvExportError) throw error
    if (isAxiosError(error)) {
      const status = error.response?.status
      if (status === 401 || status === 403)
        throw new UsageCsvExportError('forbidden')
      if (status === 429) throw new UsageCsvExportError('busy')
      if (status === 504 || error.code === 'ECONNABORTED')
        throw new UsageCsvExportError('timeout')
      if (status === 400) {
        const data: unknown = error.response?.data
        if (data instanceof ArrayBuffer && data.byteLength <= 16384) {
          try {
            const detail: unknown = JSON.parse(new TextDecoder().decode(data))
            if (
              typeof detail === 'object' &&
              detail !== null &&
              'code' in detail &&
              detail.code === 'USAGE_EXPORT_TOO_DENSE'
            ) {
              throw new UsageCsvExportError('too_dense')
            }
          } catch (parseError) {
            if (parseError instanceof UsageCsvExportError) throw parseError
          }
        }
        throw new UsageCsvExportError('invalid')
      }
    }
    throw new UsageCsvExportError('unavailable')
  } finally {
    signal?.removeEventListener('abort', cancel)
  }
}
