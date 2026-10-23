/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

const counter = z.number().int().nonnegative().safe()
const bucketShape = {
  id: z.number().optional(),
  user_id: z.number().optional(),
  username: z.string().optional(),
  model_name: z.string().optional(),
  created_at: z.number(),
  token_used: counter.nullable().optional(),
  cache_read_tokens: counter.nullable().optional(),
  cache_creation_tokens: counter.nullable().optional(),
  cache_creation_tokens_12h: counter.nullable().optional(),
  count: counter.optional(),
  quota: z.number().nullable().optional(),
  quota_usd: z.string().optional(),
  quota_money_available: z.boolean().optional(),
  quota_money_basis: z.string().optional(),
  output_image_count: counter.nullable().optional(),
  token_count_scope: z.literal('settled_text_requests').optional(),
  usage_scope: z
    .enum(['actor_personal_key', 'actor_personal_usage'])
    .optional(),
  settlement_scope: z.literal('settled_requests').optional(),
  platform_sponsored: z.boolean().optional(),
  payer_kind: z.string().optional(),
  subscription_funded: z.boolean().optional(),
  gross_settled_credits: counter.optional(),
  refunded_credits: counter.optional(),
  net_consumption_credits: counter.optional(),
  billing_basis: z.string().optional(),
  billing_currency: z.string().optional(),
  wallet_debit_supported: z.literal(false).optional(),
}

const nativeBucketSchema = z
  .object({
    ...bucketShape,
    user_id: counter.positive(),
    model_name: z.string().min(1),
    created_at: counter,
    token_used: counter.nullable(),
    count: counter.positive(),
    quota: counter,
    quota_usd: z.string().regex(/^(?:0|[1-9]\d*)\.\d{6}$/),
    quota_money_available: z.literal(true),
    quota_money_basis: z.literal('captured_credits_per_usd'),
    output_image_count: counter,
    token_count_scope: z.literal('settled_text_requests'),
    usage_scope: z.enum(['actor_personal_key', 'actor_personal_usage']),
    settlement_scope: z.literal('settled_requests'),
    platform_sponsored: z.boolean(),
    payer_kind: z.enum(['personal', 'team']),
    subscription_funded: z.boolean(),
    gross_settled_credits: counter,
    refunded_credits: counter,
    net_consumption_credits: counter,
    billing_basis: z.literal('net_consumption'),
    billing_currency: z.literal('USD'),
    wallet_debit_supported: z.literal(false),
  })
  .refine(
    (bucket) =>
      bucket.quota === bucket.net_consumption_credits &&
      bucket.gross_settled_credits - bucket.refunded_credits === bucket.quota &&
      (bucket.token_used !== null || bucket.output_image_count > 0) &&
      (bucket.token_used !== null ||
        [
          bucket.cache_read_tokens,
          bucket.cache_creation_tokens,
          bucket.cache_creation_tokens_12h,
        ].every((value) => value === null || value === undefined)),
    'Invalid native consumption receipt'
  )

/** Shared by dashboard, profile and assistant. Never coerce absent usage to zero. */
export const usageBucketSchema = z
  .object(bucketShape)
  .superRefine((bucket, ctx) => {
    const read = bucket.cache_read_tokens
    const write = bucket.cache_creation_tokens
    const twelveHour = bucket.cache_creation_tokens_12h
    if (
      (typeof bucket.token_used === 'number' &&
        (read ?? 0) + (write ?? 0) > bucket.token_used) ||
      (typeof twelveHour === 'number' &&
        ((typeof write === 'number' && twelveHour > write) ||
          (typeof bucket.token_used === 'number' &&
            twelveHour > bucket.token_used)))
    ) {
      ctx.addIssue({ code: 'custom', message: 'Invalid cache usage totals' })
    }
    const native =
      bucket.token_count_scope !== undefined ||
      bucket.settlement_scope !== undefined ||
      bucket.output_image_count !== undefined ||
      bucket.platform_sponsored !== undefined ||
      bucket.wallet_debit_supported !== undefined
    if (native && !nativeBucketSchema.safeParse(bucket).success) {
      ctx.addIssue({ code: 'custom', message: 'Invalid native usage bucket' })
    }
  })

export type UsageBucket = z.infer<typeof usageBucketSchema>
export const usageBucketsSchema = z.array(usageBucketSchema)

export function parseUsageBuckets(input: unknown): UsageBucket[] {
  return usageBucketsSchema.parse(input)
}
