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
import type { QuotaDataItem } from '@/features/dashboard/types'

/**
 * Safe division: handles NaN and Infinity cases
 */
export function safeDivide(
  value: number,
  divisor: number,
  precision: number = 3
): number {
  const result = value / divisor
  if (Number.isNaN(result) || !Number.isFinite(result)) return 0
  const factor = Math.pow(10, precision)
  return Math.round(result * factor) / factor
}

/** Cache counters are subsets of input; they never add to total token usage. */
function sumCache(
  data: QuotaDataItem[],
  field: 'cache_read_tokens' | 'cache_creation_tokens'
): number | null {
  let total = 0
  let measured = data.length === 0
  for (const item of data) {
    const value = item[field]
    const imageOnly =
      item.token_used === null &&
      item.token_count_scope === 'settled_text_requests' &&
      typeof item.output_image_count === 'number' &&
      Number.isSafeInteger(item.output_image_count) &&
      item.output_image_count > 0
    if (imageOnly && (value === null || value === undefined)) continue
    if (
      imageOnly ||
      typeof value !== 'number' ||
      !Number.isSafeInteger(value) ||
      value < 0
    )
      return null
    measured = true
    total += value
    if (!Number.isSafeInteger(total)) return null
  }
  return measured ? total : null
}

/**
 * Calculate aggregated statistics from quota data
 */
export function calculateDashboardStats(data: QuotaDataItem[]) {
  let knownQuota = 0
  let totalCount = 0
  let knownTokens = 0
  let knownOutputImages = 0
  let hasKnownTokens = data.length === 0
  let hasUnknownTokens = false
  let hasUnknownOutputImages = false
  let hasUnknownQuota = false

  for (const item of data) {
    if (typeof item.quota === 'number' && Number.isSafeInteger(item.quota)) {
      knownQuota += item.quota
    } else {
      hasUnknownQuota = true
    }
    totalCount += Number(item.count) || 0

    const tokens = item.token_used
    const outputImages = item.output_image_count
    const hasKnownOutputImages =
      typeof outputImages === 'number' &&
      Number.isSafeInteger(outputImages) &&
      outputImages >= 0

    if (
      typeof tokens === 'number' &&
      Number.isSafeInteger(tokens) &&
      tokens >= 0
    ) {
      knownTokens += tokens
      hasKnownTokens = true
    } else if (
      tokens !== null ||
      item.token_count_scope !== 'settled_text_requests' ||
      !hasKnownOutputImages ||
      outputImages === 0
    ) {
      hasUnknownTokens = true
    }

    if (hasKnownOutputImages) {
      knownOutputImages += outputImages
    } else {
      hasUnknownOutputImages = true
    }
  }

  const tokenCountScope: QuotaDataItem['token_count_scope'] =
    data.length > 0 &&
    data.every((item) => item.token_count_scope === 'settled_text_requests')
      ? 'settled_text_requests'
      : undefined

  return {
    totalQuota:
      !hasUnknownQuota && Number.isSafeInteger(knownQuota) ? knownQuota : null,
    totalCount,
    totalTokens:
      hasKnownTokens && !hasUnknownTokens && Number.isSafeInteger(knownTokens)
        ? knownTokens
        : null,
    totalCacheReadTokens: sumCache(data, 'cache_read_tokens'),
    totalCacheCreationTokens: sumCache(data, 'cache_creation_tokens'),
    totalOutputImages:
      !hasUnknownOutputImages && Number.isSafeInteger(knownOutputImages)
        ? knownOutputImages
        : null,
    tokenCountScope,
    hasOutputImages: data.some((item) => item.output_image_count !== undefined),
    hasPlatformSponsoredUsage: data.some(
      (item) => item.platform_sponsored === true
    ),
  }
}
