/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import type { UsageLog } from '../data/schema'
import type { LogOtherData } from '../types'

type UsageQuantity =
  | { kind: 'image'; count: number | null }
  | { kind: 'tokens'; input: number | null; output: number | null }

/** A measured subset of total writes, never a guess from the request TTL. */
export function cacheWrite12hQuantity(
  other: LogOtherData | null,
  tokenUsageAvailable?: boolean
): number | null {
  const total = other?.cache_creation_tokens
  const twelve = other?.cache_creation_tokens_12h
  if (
    tokenUsageAvailable === false ||
    other?.token_usage_available === false ||
    typeof total !== 'number' ||
    !Number.isSafeInteger(total) ||
    total < 0 ||
    typeof twelve !== 'number' ||
    !Number.isSafeInteger(twelve) ||
    twelve < 0 ||
    twelve > total
  )
    return null
  return twelve
}

export function logUsageQuantity(
  log: UsageLog,
  other: LogOtherData | null
): UsageQuantity {
  if (
    log.pricing_dimension === 'image' ||
    other?.pricing_dimension === 'image'
  ) {
    const count =
      log.output_image_count !== undefined
        ? log.output_image_count
        : other?.output_image_count
    return {
      kind: 'image',
      count:
        typeof count === 'number' && Number.isSafeInteger(count) && count > 0
          ? count
          : null,
    }
  }
  const unavailable =
    log.token_usage_available === false ||
    other?.token_usage_available === false
  return {
    kind: 'tokens',
    input: unavailable ? null : log.prompt_tokens,
    output: unavailable ? null : log.completion_tokens,
  }
}
