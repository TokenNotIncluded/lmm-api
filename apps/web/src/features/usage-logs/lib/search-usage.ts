/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import {
  searchProviders,
  searchQuantityLabel,
  searchUnitLabels,
  searchUnitMatchesProvider,
  type SearchBillingUnit,
  type SearchProvider,
} from '@/lib/search-metering'

import type { LogOtherData } from '../types'

const object = (value: unknown): Record<string, unknown> | null =>
  typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null

export type SearchQuantity = { label: string; count: number | null }

/** Only durable, internally consistent measurements can display a quantity. */
export function searchUsageQuantities(
  other: LogOtherData | null,
  tokenUsageAvailable?: boolean
): SearchQuantity[] {
  const billing = object(other?.search_billing)
  if (!billing) return []
  const provider = billing.provider as SearchProvider
  const unit = billing.unit as SearchBillingUnit
  if (
    !searchProviders.includes(provider) ||
    !Object.hasOwn(searchUnitLabels, unit) ||
    !searchUnitMatchesProvider(provider, unit)
  ) {
    return [{ label: 'Search usage', count: null }]
  }
  let usage = object(billing.usage)
  const fields = [
    'calls',
    'queries',
    'grounded_requests',
    'x_posts_fetched',
    'x_users_fetched',
  ] as const
  if (
    tokenUsageAvailable === false ||
    other?.token_usage_available === false ||
    usage?.provider !== provider ||
    fields.some((field) => {
      const value = usage?.[field]
      return (
        value !== undefined &&
        value !== null &&
        (typeof value !== 'number' ||
          !Number.isSafeInteger(value) ||
          value < 0 ||
          value > (field === 'grounded_requests' ? 1 : 10_000))
      )
    })
  ) {
    usage = null
  }
  const positive = (field: string) => {
    const value = usage?.[field]
    return typeof value === 'number' && value > 0
  }
  if (
    (provider !== 'xai_responses_v1' &&
      (positive('x_posts_fetched') || positive('x_users_fetched'))) ||
    (provider === 'gemini_grounding_v1' &&
      (positive('calls') ||
        (usage?.grounded_requests === 0 && positive('queries')))) ||
    (provider === 'zhipu_chat_v1' &&
      (positive('calls') || positive('queries'))) ||
    (provider !== 'gemini_grounding_v1' &&
      provider !== 'zhipu_chat_v1' &&
      (positive('queries') || positive('grounded_requests')))
  ) {
    usage = null
  }
  const measured = (field: string): number | null => {
    const value = usage?.[field]
    return typeof value === 'number' ? value : null
  }
  const primary = {
    call: 'calls',
    query: 'queries',
    grounded_request: 'grounded_requests',
  }[unit]
  const result: SearchQuantity[] = [
    { label: searchQuantityLabel(provider, unit), count: measured(primary) },
  ]
  if (provider === 'xai_responses_v1') {
    if (object(billing.x_posts_fetched_usd_per_unit))
      result.push({
        label: 'X posts fetched',
        count: measured('x_posts_fetched'),
      })
    if (object(billing.x_users_fetched_usd_per_unit))
      result.push({
        label: 'X users fetched',
        count: measured('x_users_fetched'),
      })
  }
  return result
}
