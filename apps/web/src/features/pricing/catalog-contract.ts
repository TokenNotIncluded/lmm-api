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

const rateBasis = z.enum([
  'base_group_rate',
  'effective_user_rate',
  'effective_group_rate',
])
const accountMultiplier = z
  .object({
    numerator: z.string().regex(/^[1-9]\d{0,19}$/),
    denominator: z.string().regex(/^[1-9]\d{0,19}$/),
  })
  .refine(({ numerator, denominator }) => {
    if (
      !/^[1-9]\d{0,19}$/.test(numerator) ||
      !/^[1-9]\d{0,19}$/.test(denominator)
    ) {
      return false
    }
    const n = BigInt(numerator)
    const d = BigInt(denominator)
    return n <= d && n <= 18_446_744_073_709_551_615n && d <= 10n ** 18n
  }, 'Invalid account price multiplier')
const userPrice = z.discriminatedUnion('status', [
  z.object({ status: z.literal('not_evaluated') }).strict(),
  z
    .object({
      status: z.literal('verified'),
      multiplier: accountMultiplier,
    })
    .strict(),
])

export const nativeImageVariantSchema = z
  .object({
    endpoint: z.enum(['generations', 'edits']),
    rate_basis: z.enum(['base_group_rate', 'effective_user_rate']).optional(),
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
  rate_basis: rateBasis,
  group_multiplier: z.number().finite().nonnegative(),
  input: amount,
  output: amount,
  cache_read: amount,
  cache_write: z.null(),
  request: z.null(),
  final_cost_depends_on_usage: z.literal(true),
})

export const nativeUpstreamCostPriceSchema = z
  .object({
    currency: z.literal('USD'),
    cost_source: z.literal('usage.cost'),
    credits_per_usd: z.literal(500_000),
    rounding: z.literal('ceil_total_credits'),
    rate_basis: rateBasis,
    multiplier: fraction.refine(
      ({ numerator, denominator }) =>
        /^[1-9]\d{0,127}$/.test(numerator) &&
        /^[1-9]\d{0,127}$/.test(denominator) &&
        BigInt(numerator) <= BigInt(denominator) * 1000n,
      'Invalid upstream cost multiplier'
    ),
  })
  .strict()

const unavailableReason = z.enum([
  'upstream_budget_contract_required',
  'routing_unavailable',
])

export const nativePricingSchema = z
  .object({
    schema_version: z.literal(1),
    visibility: z.enum(['public', 'session']),
    observed_at: safeInteger,
    credits_per_usd: z.literal(500_000),
    rate_basis: rateBasis,
    // Only historical effective_group_rate documents may omit this field.
    user_price: userPrice.optional(),
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
          upstream_cost_price: nativeUpstreamCostPriceSchema
            .nullable()
            .optional(),
          unavailable_reason: unavailableReason.nullable().optional(),
          image_variants: z.array(nativeImageVariantSchema).max(4096),
        })
      )
      .max(4096),
  })
  .superRefine((document, context) => {
    const publicPrice =
      document.visibility === 'public' &&
      document.rate_basis === 'base_group_rate' &&
      document.user_price?.status === 'not_evaluated'
    const accountPrice =
      document.visibility === 'session' &&
      document.rate_basis === 'effective_user_rate' &&
      document.user_price?.status === 'verified'
    const legacyPrice =
      document.rate_basis === 'effective_group_rate' &&
      (document.user_price === undefined ||
        document.user_price.status === 'not_evaluated')
    if (!publicPrice && !accountPrice && !legacyPrice) {
      context.addIssue({
        code: 'custom',
        path: ['user_price'],
        message: 'Price basis, visibility and account pricing disagree',
      })
    }
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
      if (
        model.upstream_cost_price &&
        (model.token_price !== null ||
          (model.billing_status === 'configured' &&
            model.image_variants.length === 0) ||
          model.upstream_cost_price.rate_basis !== document.rate_basis)
      ) {
        context.addIssue({
          code: 'custom',
          path: ['models', index, 'upstream_cost_price'],
          message: 'Upstream costs require a matching dynamic price basis',
        })
      }
      if (
        model.unavailable_reason &&
        model.billing_status !== 'unavailable' &&
        !(model.upstream_cost_price && model.image_variants.length > 0)
      ) {
        context.addIssue({
          code: 'custom',
          path: ['models', index, 'unavailable_reason'],
          message: 'An unavailable reason cannot describe an available route',
        })
      }
      if (
        model.token_price &&
        model.token_price.rate_basis !== document.rate_basis
      ) {
        context.addIssue({
          code: 'custom',
          path: ['models', index, 'token_price', 'rate_basis'],
          message: 'Token prices must use the document price basis',
        })
      }
      for (const [variantIndex, variant] of model.image_variants.entries()) {
        if (
          (document.rate_basis !== 'effective_group_rate' &&
            variant.rate_basis === undefined) ||
          (variant.rate_basis === 'effective_user_rate' && !accountPrice)
        ) {
          context.addIssue({
            code: 'custom',
            path: [
              'models',
              index,
              'image_variants',
              variantIndex,
              'rate_basis',
            ],
            message: 'Image price basis must identify the applicable rates',
          })
        }
      }
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
export type NativePricingContext = Pick<
  NativePricingDocument,
  'rate_basis' | 'user_price'
>
export type NativeImageVariant = z.infer<typeof nativeImageVariantSchema>
export type NativeUpstreamCostPrice = z.infer<
  typeof nativeUpstreamCostPriceSchema
>
export type NativeUnavailableReason = z.infer<typeof unavailableReason>
