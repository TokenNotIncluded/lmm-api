/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { usageLogSchema } from '../data/schema'
import type { LogCategory } from '../types'

const integer = z.number().int().safe()
const count = integer.nonnegative()
const pagination = {
  total: count,
  page: count.positive(),
  page_size: count.positive(),
}
const money = {
  quota_usd: z.string().optional(),
  quota_money_available: z.boolean(),
  quota_money_basis: z.string(),
}

export const historicalLogSchema = z.object({
  source_log_id: integer,
  user_id: count,
  token_id: count.nullable(),
  created_at: count,
  type: integer,
  username: z.string(),
  token_name: z.string(),
  model_name: z.string(),
  quota: integer.nullable(),
  prompt_tokens: integer.nullable(),
  completion_tokens: integer.nullable(),
  group: z.string(),
  request_id: z.string(),
  channel: count.optional(),
  upstream_request_id: z.string().optional(),
  ...money,
})

export type HistoricalLog = z.infer<typeof historicalLogSchema>

const historicalPageSchema = z.object({
  ...pagination,
  source: z.literal('legacy_log_only'),
  items: z.array(historicalLogSchema),
})
const commonPageSchema = z.object({
  ...pagination,
  source: z.string().optional(),
  items: z.array(
    usageLogSchema.extend({ token_id: count.nullish().default(null) })
  ),
})
const drawingLogSchema = z.object({
  id: count,
  user_id: count,
  channel_id: count,
  code: integer,
  mj_id: z.string(),
  action: z.string(),
  submit_time: count,
  finish_time: count.optional(),
  start_time: count.optional(),
  fail_reason: z.string().optional(),
  progress: z.string(),
  prompt: z.string(),
  prompt_en: z.string().optional(),
  description: z.string().optional(),
  buttons: z.string().optional(),
  properties: z.string().optional(),
  image_url: z.string().optional(),
  video_url: z.string().optional(),
  video_urls: z.string().optional(),
  status: z.string(),
  other: z.string().optional(),
  created_at: count.optional(),
  updated_at: count.optional(),
})
const taskLogSchema = z.object({
  id: count,
  user_id: count,
  username: z.string().optional(),
  platform: z.string(),
  task_id: z.string(),
  action: z.string(),
  channel_id: count,
  submit_time: count,
  finish_time: count.optional(),
  progress: z.string().optional(),
  progress_message_en: z.string().optional(),
  data: z.unknown().optional(),
  result_url: z.string().optional(),
  fail_reason: z.string().optional(),
  status: z.string(),
  other: z.string().optional(),
  created_at: count.optional(),
  updated_at: count.optional(),
})
const drawingPageSchema = z.object({
  ...pagination,
  items: z.array(drawingLogSchema),
})
const taskPageSchema = z.object({
  ...pagination,
  items: z.array(taskLogSchema),
})
const envelope = z.object({ success: z.literal(true), data: z.unknown() })
const historicalSource = z.object({ source: z.literal('legacy_log_only') })

export type LogsPage =
  | (z.infer<typeof commonPageSchema> & { kind: 'common' })
  | (z.infer<typeof historicalPageSchema> & { kind: 'historical' })
  | (z.infer<typeof drawingPageSchema> & { kind: 'drawing' })
  | (z.infer<typeof taskPageSchema> & { kind: 'task' })

/** Missing data, failed reads and malformed rows must not become an empty page. */
export function parseLogsPage(value: unknown, category: LogCategory): LogsPage {
  const response = envelope.safeParse(value)
  if (!response.success) throw new Error('Usage history response unavailable')
  const data = response.data.data
  if (category === 'common') {
    if (historicalSource.safeParse(data).success) {
      const result = historicalPageSchema.safeParse(data)
      if (result.success) return { ...result.data, kind: 'historical' }
    } else {
      const result = commonPageSchema.safeParse(data)
      if (result.success) return { ...result.data, kind: 'common' }
    }
  } else if (category === 'drawing') {
    const result = drawingPageSchema.safeParse(data)
    if (result.success) return { ...result.data, kind: 'drawing' }
  } else {
    const result = taskPageSchema.safeParse(data)
    if (result.success) return { ...result.data, kind: 'task' }
  }
  throw new Error('Usage history response unavailable')
}

const unavailableSchema = z.object({
  success: z.literal(false),
  code: z.literal('USAGE_HISTORY_UNAVAILABLE'),
  availability: z.literal('unavailable'),
  category: z.enum(['drawing', 'task']),
})

export class UsageHistoryUnavailableError extends Error {
  constructor(readonly category: 'drawing' | 'task') {
    super('This history category is unavailable on this server')
    this.name = 'UsageHistoryUnavailableError'
  }
}

export function parseHistoryUnavailable(value: unknown, category: LogCategory) {
  const result = unavailableSchema.safeParse(value)
  return result.success && result.data.category === category
    ? new UsageHistoryUnavailableError(result.data.category)
    : undefined
}
