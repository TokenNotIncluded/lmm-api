/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import type { LogOtherData } from '../types'

const object = (value: unknown): Record<string, unknown> | null =>
  typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
const count = (value: unknown): number | null =>
  typeof value === 'number' &&
  Number.isSafeInteger(value) &&
  value >= 0 &&
  value <= 16_000_000
    ? value
    : null

export function embeddingUsageQuantities(
  other: LogOtherData | null,
  available?: boolean
): { label: string; count: number | null }[] {
  const billing = object(other?.embedding_billing)
  if (!billing) return []
  const usage = object(billing.usage)
  let text = count(usage?.text_tokens)
  let image = count(usage?.image_tokens)
  const total = count(usage?.total_input_tokens)
  if (
    available === false ||
    other?.token_usage_available === false ||
    !['ali_separate_input_v1', 'ali_inclusive_input_v1'].includes(
      String(billing.provider)
    ) ||
    billing.provider !== usage?.provider ||
    text === null ||
    image === null ||
    total === null ||
    text + image !== total
  ) {
    text = null
    image = null
  }
  return [
    { label: 'Embedding text input tokens', count: text },
    { label: 'Embedding image and video input tokens', count: image },
  ]
}
