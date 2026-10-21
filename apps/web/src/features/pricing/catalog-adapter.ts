/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import type { NativePricingDocument } from './catalog-contract'
import type { PricingData, PricingGroupPrices, PricingModel } from './types'

const endpoints: Record<
  string,
  { type: string; path?: string; method?: string }
> = {
  'openai-completions': {
    type: 'openai',
    path: '/v1/chat/completions',
    method: 'POST',
  },
  'openai-responses': {
    type: 'openai-response',
    path: '/v1/responses',
    method: 'POST',
  },
  'anthropic-messages': {
    type: 'anthropic',
    path: '/v1/messages',
    method: 'POST',
  },
  'google-generative-ai': {
    type: 'gemini',
    path: '/v1beta/models/{model}:generateContent',
    method: 'POST',
  },
  'system-one': { type: 'systemone', path: '/v1/systemone', method: 'POST' },
  'openai-images-generations': {
    type: 'image-generation',
    path: '/v1/images/generations',
    method: 'POST',
  },
  // Image edits use multipart input; do not copy a JSON generation example.
  'openai-images-edits': { type: 'image-edits' },
}

function groupPrices(
  row: NativePricingDocument['models'][number]
): PricingGroupPrices {
  const token = row.billing_status === 'configured' ? row.token_price : null
  return {
    input_price: token?.input ?? null,
    output_price: token?.output ?? null,
    cache_read_price: token?.cache_read ?? null,
    cache_write_price: token?.cache_write ?? null,
    model_price: null,
    image_price: null,
    audio_input_price: null,
    audio_output_price: null,
  }
}

/** A shared headline rate exists only when every visible group agrees. */
function commonPrice(
  prices: PricingGroupPrices[],
  field: keyof PricingGroupPrices
): number | null {
  const value = prices[0]?.[field] ?? null
  return prices.every((price) => price[field] === value) ? value : null
}

export function adaptNativePricing(
  document: NativePricingDocument
): PricingData {
  const pricingContext = {
    rate_basis: document.rate_basis,
    user_price: document.user_price,
  }
  const grouped = new Map<string, NativePricingDocument['models']>()
  for (const row of document.models) {
    const rows = grouped.get(row.model) ?? []
    rows.push(row)
    grouped.set(row.model, rows)
  }
  const models: PricingModel[] = []
  const endpointMap: PricingData['supported_endpoint'] = {}
  for (const [name, rows] of grouped) {
    const tokenRows = rows.filter((row) => row.token_price !== null)
    const prices = rows.map(groupPrices)
    const apis = new Set<string>()
    for (const row of rows) {
      for (const api of row.apis) {
        const endpoint = endpoints[api]
        if (!endpoint) continue
        apis.add(endpoint.type)
        if (endpoint.path && endpoint.method) {
          endpointMap[endpoint.type] = {
            path: endpoint.path,
            method: endpoint.method,
          }
        }
      }
    }
    models.push({
      pricing_context: pricingContext,
      id: models.length + 1,
      key: name,
      model_name: name,
      pricing_schema_version: 2,
      pricing_currency: 'USD',
      pricing_version: String(document.pricing_version),
      quota_type: tokenRows.length ? 0 : 2,
      billing_mode: 'ratio',
      input_price: commonPrice(prices, 'input_price'),
      output_price: commonPrice(prices, 'output_price'),
      cache_read_price: commonPrice(prices, 'cache_read_price'),
      cache_write_price: null,
      group_prices: tokenRows.length
        ? Object.fromEntries(rows.map((row) => [row.group, groupPrices(row)]))
        : undefined,
      group_endpoints: Object.fromEntries(
        rows.map((row) => [
          row.group,
          row.apis.flatMap((api) => endpoints[api]?.type ?? []),
        ])
      ),
      group_quota_types: Object.fromEntries(
        rows.map((row) => [row.group, row.token_price ? 0 : 2])
      ),
      image_variants: rows.flatMap((row) =>
        row.image_variants.map((variant) => ({
          ...variant,
          group: row.group,
          model: row.model,
        }))
      ),
      enable_groups: rows.map((row) => row.group),
      supported_endpoint_types: [...apis],
      group_ratio: Object.fromEntries(rows.map((row) => [row.group, 1])),
      runtime_state: {
        status: rows.every((row) => row.billing_status === 'unavailable')
          ? 'unavailable'
          : 'unknown',
        source: 'routing_configuration',
        observed_at: document.observed_at,
      },
      performance_available: false,
    })
  }
  return {
    pricing_context: pricingContext,
    success: true,
    pricing_schema_version: 2,
    pricing_currency: 'USD',
    credits_per_usd: document.credits_per_usd,
    data: models,
    vendors: [],
    group_ratio: Object.fromEntries(document.groups.map((group) => [group, 1])),
    usable_group: Object.fromEntries(
      document.groups.map((group) => [group, { desc: group, ratio: 1 }])
    ),
    auto_groups: [],
    supported_endpoint: endpointMap,
    runtime_available: true,
    performance_available: false,
  }
}
