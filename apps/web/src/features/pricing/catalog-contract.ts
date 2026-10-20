/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { z } from 'zod'

const safeInteger = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER)
const amount = z
  .number()
  .finite()
  .nonnegative()
  .max(Number.MAX_SAFE_INTEGER)
  .nullable()
const identifier = z.string().min(1).max(1024)
const fraction = z.object({
  numerator: z.string().regex(/^(0|[1-9]\d{0,127})$/),
  denominator: z.string().regex(/^[1-9]\d{0,127}$/),
})

export const nativeImageVariantSchema = z
  .object({
    endpoint: z.enum(['generations', 'edits']),
    size: identifier,
    quality: identifier,
    max_n: safeInteger.positive(),
    count_basis: z.enum(['requested', 'returned_payloads']),
    usd_micros_per_image: fraction,
    credits_per_image: fraction,
    reservation_credits_for_one: safeInteger,
    rounding: z.literal('ceil_total_credits'),
    pricing_version: safeInteger,
  })
  .refine((variant) => {
    const micros = variant.usd_micros_per_image
    const credits = variant.credits_per_image
    return (
      BigInt(micros.numerator) * 500_000n * BigInt(credits.denominator) ===
      BigInt(credits.numerator) * 1_000_000n * BigInt(micros.denominator)
    )
  }, 'Image currency amounts disagree')

export const nativeTokenPriceSchema = z.object({
  currency: z.literal('USD'),
  unit: z.literal('million_tokens'),
  rate_basis: z.literal('effective_group_rate'),
  group_multiplier: z.number().finite().nonnegative(),
  input: amount,
  output: amount,
  cache_read: amount,
  cache_write: z.null(),
  request: z.null(),
  final_cost_depends_on_usage: z.literal(true),
})

export const nativePricingSchema = z
  .object({
    schema_version: z.literal(1),
    visibility: z.enum(['public', 'session']),
    observed_at: safeInteger,
    credits_per_usd: z.literal(500_000),
    rate_basis: z.literal('effective_group_rate'),
    invocation_authorized: z.literal(false),
    routing_version: safeInteger,
    pricing_version: safeInteger,
    image_pricing_version: safeInteger.nullable(),
    groups: z.array(identifier).max(4096),
    models: z
      .array(
        z.object({
          id: identifier,
          model: identifier,
          group: identifier,
          apis: z.array(identifier).max(32),
          billing_status: z.enum([
            'configured',
            'usage_dependent',
            'unavailable',
          ]),
          token_price: nativeTokenPriceSchema.nullable(),
          image_variants: z.array(nativeImageVariantSchema).max(4096),
        })
      )
      .max(4096),
  })
  .superRefine((document, context) => {
    const groups = new Set(document.groups)
    const models = new Set<string>()
    for (const [index, model] of document.models.entries()) {
      const key = JSON.stringify([model.group, model.model])
      if (!groups.has(model.group) || models.has(key)) {
        context.addIssue({
          code: 'custom',
          path: ['models', index],
          message: 'Invalid catalog group or duplicate model',
        })
      }
      models.add(key)
    }
  })

export const nativeRuntimeSchema = z.object({
  schema_version: z.literal(1),
  visibility: z.enum(['public', 'session']),
  observed_at: safeInteger,
  data: z.record(
    z.string(),
    z.object({
      status: z.enum(['unknown', 'unavailable', 'no_access', 'not_listed']),
      source: z.enum(['routing_configuration', 'account_permissions']),
      observed_at: safeInteger,
    })
  ),
})

export type NativePricingDocument = z.infer<typeof nativePricingSchema>
export type NativeImageVariant = z.infer<typeof nativeImageVariantSchema>
