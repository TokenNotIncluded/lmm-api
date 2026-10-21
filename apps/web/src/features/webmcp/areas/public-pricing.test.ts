/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import type { NativePricingDocument } from '@/features/pricing/catalog-contract'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { publicToolsTools } from './public-tools'

const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
})

const tools = publicToolsTools({
  router: { navigate: async () => undefined, subscribe: () => () => undefined },
})
const estimate = tools.find((tool) => tool.name === 'lmm_pricing_estimate')
assert.ok(estimate)

function native(input: number | null = 2): NativePricingDocument {
  return {
    schema_version: 1,
    visibility: 'public',
    observed_at: 100,
    credits_per_usd: 500_000,
    rate_basis: 'effective_group_rate',
    invocation_authorized: false,
    routing_version: 1,
    pricing_version: 1,
    image_pricing_version: null,
    groups: ['standard'],
    models: [
      {
        id: 'synthetic-model',
        model: 'exact-model',
        group: 'standard',
        apis: ['openai-completions'],
        billing_status: 'configured',
        image_variants: [],
        token_price: {
          currency: 'USD',
          unit: 'million_tokens',
          rate_basis: 'effective_group_rate',
          group_multiplier: 5,
          input,
          output: 6,
          cache_read: 0,
          cache_write: null,
          request: null,
          final_cost_depends_on_usage: true,
        },
      },
    ],
  }
}

function serve(document: NativePricingDocument) {
  useAuthStore.getState().auth.reset('complete')
  api.defaults.adapter = async (config) => ({
    data: document,
    status: 200,
    statusText: 'OK',
    headers: {},
    config,
  })
}

test('WebMCP estimate uses exact model and effective rates and converts USD to Credits once', async () => {
  serve(native())
  const result = await estimate.execute(
    {
      model: 'exact-model',
      group: 'standard',
      input_tokens: 1_000_000,
      output_tokens: 0,
    },
    { signal: new AbortController().signal }
  )
  assert.deepEqual(result, {
    page: '/pricing',
    rate_basis: 'effective_group_rate',
    user_price: null,
    model: 'exact-model',
    vendor: null,
    group: 'standard',
    group_multiplier: 1,
    input_tokens: 1_000_000,
    output_tokens: 0,
    cached_tokens: 0,
    estimated_usd: 2,
    estimated_credit: 1_000_000,
    credits_per_usd: 500_000,
    estimable: true,
    billing_basis:
      'Displayed USD rates already include applicable account and group factors; they are converted at 500000 Credits per USD without applying discounts again. This text-only estimate is not a locked quote or a settlement receipt.',
  })
  await assert.rejects(
    estimate.execute(
      { model: 'EXACT-MODEL', input_tokens: 1, output_tokens: 0 },
      { signal: new AbortController().signal }
    ),
    /Unknown model/
  )
  await assert.rejects(
    estimate.execute(
      {
        model: 'exact-model',
        group: 'unlisted',
        input_tokens: 1,
        output_tokens: 0,
      },
      { signal: new AbortController().signal }
    ),
    /not listed for this group/
  )
})

test('WebMCP account estimate preserves verified factor and uses the already discounted rate once', async () => {
  const document = native(1.94)
  document.visibility = 'session'
  document.rate_basis = 'effective_user_rate'
  document.user_price = {
    status: 'verified',
    multiplier: { numerator: '97', denominator: '100' },
  }
  const token = document.models[0]?.token_price
  assert.ok(token)
  token.rate_basis = 'effective_user_rate'
  serve(document)
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-price-session',
    token_type: 'Bearer',
    access_expires_at: 4_000_000_000,
    user: { id: 7, username: 'synthetic', role: 1 },
    session: {
      sid: 'synthetic-price-session',
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 4_000_000_000,
    },
  })
  const result = await estimate.execute(
    {
      model: 'exact-model',
      group: 'standard',
      input_tokens: 1_000_000,
      output_tokens: 0,
    },
    { signal: new AbortController().signal }
  )
  assert.ok(typeof result === 'object' && result !== null)
  assert.ok(
    'estimated_usd' in result &&
      'estimated_credit' in result &&
      'rate_basis' in result &&
      'user_price' in result
  )
  assert.equal(result.estimated_usd, 1.94)
  assert.equal(result.estimated_credit, 970_000)
  assert.equal(result.rate_basis, 'effective_user_rate')
  assert.deepEqual(result.user_price, document.user_price)
})

test('unknown and unsafe large estimates are unavailable and explicit free input remains zero', async () => {
  for (const input of [null, Number.MAX_SAFE_INTEGER, 0]) {
    serve(native(input))
    const result = await estimate.execute(
      { model: 'exact-model', input_tokens: 20_000_000, output_tokens: 0 },
      { signal: new AbortController().signal }
    )
    assert.ok(
      typeof result === 'object' &&
        result !== null &&
        'estimable' in result &&
        'estimated_credit' in result
    )
    assert.equal(result.estimable, input === 0)
    assert.equal(result.estimated_credit, input === 0 ? 0 : null)
  }
})
