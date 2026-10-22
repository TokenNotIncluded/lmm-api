/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

const id = z
  .string()
  .regex(/^[1-9]\d{0,18}$/)
  .refine((v) => BigInt(v) <= 9223372036854775807n)
const hash = z.string().regex(/^[a-f0-9]{64}$/)
const nonnegative = z
  .string()
  .regex(/^(0|[1-9]\d{0,18})$/)
  .refine((v) => BigInt(v) <= 9223372036854775807n)
export const fractionSchema = z
  .object({
    numerator: nonnegative,
    denominator: id.refine((v) => BigInt(v) <= 1000000000000000000n),
  })
  .strict()
export const expectedSchema = z
  .object({ routing: id, pricing: id, snapshot_sha256: hash })
  .strict()
const fxSchema = z
  .object({
    key: z.literal('USDExchangeRate'),
    revision: id,
    raw: z.string().max(128),
  })
  .strict()
  .nullable()
const rateSchema = z
  .object({
    name: z.enum([
      'input',
      'output',
      'cache_read',
      'cache_write',
      'cache_write_1h',
      'cache_write_12h',
      'request',
      'search',
      'x_posts_fetched',
      'x_users_fetched',
      'embedding_text',
      'embedding_image',
    ]),
    unit: z.enum([
      'million_tokens',
      'request',
      'search_call',
      'search_query',
      'grounded_request',
      'item',
    ]),
    up_to_input_tokens: id.nullable(),
    amount: fractionSchema,
  })
  .strict()
const priceSchema = z
  .object({
    model: z.string().min(1).max(800),
    group: z.string().min(1).max(256),
    speed: z.enum(['standard', 'fast', 'ultrafast']),
    rule_id: id,
    billing_mode: z.enum(['tokens', 'tiered_decimal', 'upstream_cost']),
    group_multiplier_ppm: z.number().int().min(1).max(1000000000),
    speed_multiplier_ppm: z.number().int().min(1).max(1000000000),
    currency: z.literal('USD'),
    prices: z.array(rateSchema).max(4096),
  })
  .strict()
export const targetSchema = z
  .object({
    target_id: id,
    upstream_id: id,
    model: z.string().min(1).max(800),
    priority: z.number().int().min(0).max(65535),
    weight: z.number().int().min(0).max(4294967295),
    enabled: z.boolean(),
  })
  .strict()
const routeSchema = z
  .object({
    route_id: id,
    model: z.string().min(1).max(800),
    group: z.string().min(1).max(256),
    targets: z.array(targetSchema).max(64),
  })
  .strict()
export const catalogSchema = z
  .object({
    contract: z.literal('core.routing-admin.v1'),
    expected: expectedSchema,
    prices: z.array(priceSchema).max(2000),
    routes: z.array(routeSchema).max(2000),
    exchange_rate: fxSchema,
    exchange_rate_available: z.boolean(),
    can_edit_prices: z.boolean(),
    can_edit_priorities: z.boolean(),
  })
  .strict()
  .superRefine((value, ctx) => {
    if (
      new Set(value.prices.map((p) => p.rule_id)).size !==
        value.prices.length ||
      new Set(value.routes.map((r) => r.route_id)).size !==
        value.routes.length ||
      value.routes.some(
        (r) =>
          new Set(r.targets.map((t) => t.target_id)).size !== r.targets.length
      )
    )
      ctx.addIssue({ code: 'custom', message: 'Duplicate routing identities' })
    if (value.exchange_rate_available && !value.exchange_rate)
      ctx.addIssue({ code: 'custom', message: 'Missing exchange rate' })
  })
export const adjustmentSchema = z.enum(['keep', 'multiply', 'divide'])
const priceChange = z
  .object({
    model: z.string(),
    group: z.string(),
    speed: z.enum(['standard', 'fast', 'ultrafast']),
    rule_id: id,
    adjustment: adjustmentSchema,
    before: priceSchema,
    source: priceSchema,
    after: priceSchema,
  })
  .strict()
const priorityChange = z
  .object({
    route_id: id,
    target_id: id,
    upstream_id: id,
    before: z.number().int().min(0).max(65535),
    after: z.number().int().min(0).max(65535),
  })
  .strict()
const previewCommon = {
  preview_id: hash,
  confirmation_sha256: hash,
  expires_at: id.refine((v) => BigInt(v) <= 253402300799n),
  expected: expectedSchema,
  next: expectedSchema,
  source_sha256: hash,
  exchange_rate: fxSchema,
}
export const previewSchema = z
  .discriminatedUnion('kind', [
    z
      .object({
        ...previewCommon,
        kind: z.literal('prices'),
        changes: z.array(priceChange).min(1).max(2000),
      })
      .strict(),
    z
      .object({
        ...previewCommon,
        kind: z.literal('priorities'),
        changes: z.array(priorityChange).min(1).max(64),
      })
      .strict(),
  ])
  .superRefine((value, ctx) => {
    if (
      BigInt(value.next.routing) !== BigInt(value.expected.routing) + 1n ||
      BigInt(value.next.pricing) !==
        BigInt(value.expected.pricing) + (value.kind === 'prices' ? 1n : 0n)
    )
      ctx.addIssue({ code: 'custom', message: 'Invalid version transition' })
    if (value.kind === 'prices')
      for (const change of value.changes) {
        const prices = [change.before, change.source, change.after]
        if (
          prices.some(
            (p) =>
              p.model !== change.model ||
              p.group !== change.group ||
              p.speed !== change.speed ||
              p.currency !== 'USD'
          ) ||
          change.before.rule_id !== change.rule_id ||
          change.after.rule_id !== change.rule_id
        )
          ctx.addIssue({
            code: 'custom',
            message: 'Inconsistent price preview',
          })
        const dimensions = (p: typeof change.before) =>
          p.prices
            .map((r) => [r.name, r.unit, r.up_to_input_tokens].join(':'))
            .join('|')
        if (
          prices.some(
            (p) =>
              p.group_multiplier_ppm !== change.before.group_multiplier_ppm ||
              p.speed_multiplier_ppm !== change.before.speed_multiplier_ppm ||
              dimensions(p) !== dimensions(change.before)
          )
        )
          ctx.addIssue({ code: 'custom', message: 'Changed pricing contract' })
        if (change.adjustment !== 'keep' && !value.exchange_rate)
          ctx.addIssue({
            code: 'custom',
            message: 'Missing frozen exchange rate',
          })
      }
  })
export const publishedSchema = z
  .object({
    operation_id: hash,
    published: expectedSchema,
    applied_on_this_node: z.boolean(),
    replayed: z.boolean(),
  })
  .strict()
export type Catalog = z.infer<typeof catalogSchema>
export type Preview = z.infer<typeof previewSchema>
export type Published = z.infer<typeof publishedSchema>
export type Expected = z.infer<typeof expectedSchema>
export type Adjustment = z.infer<typeof adjustmentSchema>
export type Price = z.infer<typeof priceSchema>
export type Rate = z.infer<typeof rateSchema>
export type Target = z.infer<typeof targetSchema>
export type PriorityChange = { target_id: string; priority: number }
