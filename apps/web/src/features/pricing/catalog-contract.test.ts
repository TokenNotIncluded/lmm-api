/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { adaptNativePricing } from './catalog-adapter'
import {
  nativePricingSchema,
  nativeRuntimeSchema,
  type NativePricingDocument,
} from './catalog-contract'
import { filterAndSortModels } from './lib/filters'
import {
  formatImageVariantCredits,
  formatImageVariantUSD,
} from './lib/image-price'
import { formatGroupPrice, formatPrice } from './lib/price'

function fixture(): NativePricingDocument {
  const token = {
    currency: 'USD',
    unit: 'million_tokens',
    rate_basis: 'effective_group_rate',
    group_multiplier: 5,
    input: 2,
    output: 6,
    cache_read: 0,
    cache_write: null,
    request: null,
    final_cost_depends_on_usage: true,
  } satisfies NonNullable<
    NativePricingDocument['models'][number]['token_price']
  >
  return {
    schema_version: 1,
    visibility: 'public',
    observed_at: 100,
    credits_per_usd: 500_000,
    rate_basis: 'effective_group_rate',
    invocation_authorized: false,
    routing_version: 2,
    pricing_version: 3,
    image_pricing_version: null,
    groups: ['standard', 'premium'],
    models: [
      {
        id: 'one',
        model: 'exact-model',
        group: 'standard',
        apis: ['openai-completions', 'system-one'],
        billing_status: 'configured',
        token_price: token,
        image_variants: [],
      },
      {
        id: 'two',
        model: 'exact-model',
        group: 'premium',
        apis: ['openai-completions'],
        billing_status: 'configured',
        token_price: { ...token, input: 4, output: 12 },
        image_variants: [],
      },
    ],
  }
}

test('native pricing parses the direct versioned document and keeps group rates separate', () => {
  const pricing = adaptNativePricing(nativePricingSchema.parse(fixture()))
  assert.equal(pricing.data.length, 1)
  const model = pricing.data[0]
  assert.ok(model)
  assert.equal(model.input_price, null)
  assert.equal(model.cache_read_price, 0)
  assert.equal(formatPrice(model, 'input', 'M', 'USD'), '-')
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'standard'), '2 USD')
  assert.equal(
    formatGroupPrice(model, 'premium', 'input', 'M', 'USD', { premium: 100 }),
    '4 USD'
  )
  assert.deepEqual(model.supported_endpoint_types, ['openai', 'systemone'])
  assert.equal(pricing.supported_endpoint.systemone?.path, '/v1/systemone')
  assert.equal(model.runtime_state?.status, 'unknown')
  assert.equal(model.performance_available, false)
})

test('selected group drives sorting and never substitutes unknown prices with zero', () => {
  const input = fixture()
  input.models.push({
    ...input.models[0]!,
    id: 'three',
    model: 'unknown',
    token_price: null,
    billing_status: 'unavailable',
  })
  const models = adaptNativePricing(nativePricingSchema.parse(input)).data
  const filtered = filterAndSortModels(models, {
    search: '',
    vendor: 'all',
    group: 'standard',
    quotaType: 'all',
    endpointType: 'all',
    tag: 'all',
    sortBy: 'price-low',
  })
  assert.equal(filtered[0]?.model_name, 'exact-model')
  assert.equal(filtered[0]?.input_price, 2)
  assert.equal(filtered[1]?.model_name, 'unknown')
})

test('token null remains unknown while an explicit zero remains free', () => {
  const input = fixture()
  const token = input.models[0]?.token_price
  assert.ok(token)
  input.models = [
    { ...input.models[0]!, token_price: { ...token, input: null, output: 0 } },
  ]
  const model = adaptNativePricing(nativePricingSchema.parse(input)).data[0]
  assert.ok(model)
  assert.equal(formatPrice(model, 'input', 'M', 'USD'), '-')
  assert.equal(formatPrice(model, 'output', 'M', 'USD'), '0 USD')
})

test('group projection cannot borrow another group protocol or token prices', () => {
  const input = fixture()
  const second = input.models[1]
  assert.ok(second)
  second.token_price = null
  second.apis = ['openai-images-generations']
  const pricing = adaptNativePricing(nativePricingSchema.parse(input))
  const model = pricing.data[0]
  assert.ok(model)
  assert.equal(model.input_price, null)
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'premium'), '-')
  const filters = {
    search: '',
    vendor: 'all',
    group: 'premium',
    quotaType: 'all',
    endpointType: 'all',
    tag: 'all',
    sortBy: 'name',
  }
  const selected = filterAndSortModels(pricing.data, filters)
  assert.equal(selected[0]?.input_price, null)
  assert.equal(selected[0]?.quota_type, 2)
  assert.deepEqual(selected[0]?.supported_endpoint_types, ['image-generation'])
  assert.equal(
    filterAndSortModels(pricing.data, { ...filters, endpointType: 'systemone' })
      .length,
    0
  )
  assert.equal(
    filterAndSortModels(pricing.data, { ...filters, search: 'systemone' })
      .length,
    0
  )
  assert.equal(
    filterAndSortModels(pricing.data, { ...filters, quotaType: 'token' })
      .length,
    0
  )
})

test('image exact ratios stay per-image and cannot become token quotes', () => {
  const input = fixture()
  const variant = {
    endpoint: 'generations',
    size: '1024x1024',
    quality: 'standard',
    max_n: 4,
    count_basis: 'returned_payloads',
    usd_micros_per_image: { numerator: '80000', denominator: '2' },
    credits_per_image: { numerator: '20000', denominator: '1' },
    reservation_credits_for_one: 20000,
    rounding: 'ceil_total_credits',
    pricing_version: 3,
  } satisfies NonNullable<
    NativePricingDocument['models'][number]
  >['image_variants'][number]
  input.models = [
    {
      id: 'image',
      model: 'image-model',
      group: 'standard',
      apis: ['openai-images-generations'],
      billing_status: 'configured',
      token_price: null,
      image_variants: [variant],
    },
  ]
  const model = adaptNativePricing(nativePricingSchema.parse(input)).data[0]
  assert.ok(model)
  const image = model.image_variants?.[0]
  assert.ok(image)
  assert.equal(model.quota_type, 2)
  assert.equal(model.input_price, null)
  assert.equal(model.image_price, undefined)
  assert.equal(formatImageVariantUSD(image), '0.04 USD')
  assert.equal(formatImageVariantCredits(image), '20000 Credits')
  input.models[0]!.image_variants[0]!.credits_per_image.numerator = '20001'
  assert.equal(nativePricingSchema.safeParse(input).success, false)
})

test('invalid denomination, unit, duplicate group model and legacy envelopes fail at the boundary', () => {
  const input = fixture()
  for (const bad of [
    { success: true, data: [] },
    { ...input, credits_per_usd: 1 },
    { ...input, invocation_authorized: true },
    { ...input, models: [...input.models, input.models[0]] },
    {
      ...input,
      models: [
        {
          ...input.models[0],
          token_price: { ...input.models[0]?.token_price, unit: 'image' },
        },
      ],
    },
  ])
    assert.equal(nativePricingSchema.safeParse(bad).success, false)
})

test('runtime projection cannot turn configuration into a live healthy badge', () => {
  const document = {
    schema_version: 1,
    visibility: 'public',
    observed_at: 100,
    data: {
      model: {
        status: 'unknown',
        source: 'routing_configuration',
        observed_at: 100,
      },
    },
  }
  assert.equal(nativeRuntimeSchema.safeParse(document).success, true)
  assert.equal(
    nativeRuntimeSchema.safeParse({
      ...document,
      data: { model: { ...document.data.model, status: 'available' } },
    }).success,
    false
  )
})
