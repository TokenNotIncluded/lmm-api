/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import type { SecurityAuditEvent } from './security-audit-types'

export function securityAuditUserFilter(
  event: Pick<SecurityAuditEvent, 'user_id' | 'username'>
): string | undefined {
  const username = event.username?.trim()
  if (username) return username
  return event.user_id && event.user_id > 0 ? String(event.user_id) : undefined
}

export function securityAuditTotalPages({
  source,
  deterministicTotal,
  aiReviewTotal,
  pageSize,
}: {
  source?: string
  deterministicTotal: number
  aiReviewTotal: number
  pageSize: number
}): number {
  const normalizedPageSize = Math.max(1, pageSize)
  const pagesFor = (total: number) =>
    Math.ceil(Math.max(0, total) / normalizedPageSize)

  if (source === 'ai_review') return Math.max(1, pagesFor(aiReviewTotal))
  if (source) return Math.max(1, pagesFor(deterministicTotal))

  // The unfiltered view reads both lanes at the same page index. Its last
  // page must therefore reach the longer lane, rather than average both
  // totals and leave the tail of AI reviews (or rule events) unreachable.
  return Math.max(1, pagesFor(Math.max(deterministicTotal, aiReviewTotal)))
}
