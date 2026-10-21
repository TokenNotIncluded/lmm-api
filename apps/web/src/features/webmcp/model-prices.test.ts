/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { installWebMcp, type ModelContextTool } from './index'

test('registered model price tool reports native group USD and Credits without legacy ratios', async () => {
  const previousDocument = globalThis.document
  const previousAuth = useAuthStore.getState().auth
  const adapter = api.defaults.adapter
  let account = false
  let readPrices: ModelContextTool | undefined
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: {
      modelContext: {
        registerTool: async (tool: ModelContextTool) => {
          if (tool.name === 'lmm_model_prices') readPrices = tool
        },
      },
    },
  })
  useAuthStore.getState().auth.reset('complete')
  api.defaults.adapter = async (config) => ({
    status: 200,
    statusText: 'OK',
    headers: {},
    config,
    data: {
      schema_version: 1,
      visibility: account ? 'session' : 'public',
      observed_at: 100,
      credits_per_usd: 500_000,
      rate_basis: account ? 'effective_user_rate' : 'base_group_rate',
      user_price: account
        ? {
            status: 'verified',
            multiplier: { numerator: '97', denominator: '100' },
          }
        : { status: 'not_evaluated' },
      invocation_authorized: false,
      routing_version: 1,
      pricing_version: 2,
      image_pricing_version: null,
      groups: ['standard'],
      models: [
        {
          id: 'synthetic',
          model: 'exact',
          group: 'standard',
          apis: ['openai-completions'],
          billing_status: 'configured',
          image_variants: [],
          token_price: {
            currency: 'USD',
            unit: 'million_tokens',
            rate_basis: account ? 'effective_user_rate' : 'base_group_rate',
            group_multiplier: 5,
            input: 2,
            output: null,
            cache_read: 0,
            cache_write: null,
            request: null,
            final_cost_depends_on_usage: true,
          },
        },
      ],
    },
  })
  const cleanup = installWebMcp({
    navigate: async () => undefined,
    subscribe: () => () => undefined,
  })
  try {
    assert.ok(readPrices)
    const output = await readPrices.execute(
      { model: 'exact' },
      { signal: new AbortController().signal }
    )
    assert.ok(
      typeof output === 'object' && output !== null && 'models' in output
    )
    assert.deepEqual(output.models, [
      {
        model: 'exact',
        vendor: null,
        quota_type: 0,
        supported_endpoints: ['openai'],
        token_prices_by_group: [
          {
            group: 'standard',
            unit: 'million_tokens',
            input_usd: 2,
            output_usd: null,
            cache_read_usd: 0,
            cache_write_usd: null,
            input_credits: 1_000_000,
            output_credits: null,
            cache_read_credits: 0,
            cache_write_credits: null,
          },
        ],
        image_variants: [],
        invocation_authorized: false,
      },
    ])
    assert.equal(JSON.stringify(output).includes('null'), true)
    assert.equal(JSON.stringify(output).includes('model_ratio'), false)
    assert.ok('rate_basis' in output && 'user_price' in output)
    assert.equal(output.rate_basis, 'base_group_rate')
    assert.deepEqual(output.user_price, { status: 'not_evaluated' })
    account = true
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
    const personal = await readPrices.execute(
      { model: 'exact' },
      { signal: new AbortController().signal }
    )
    assert.ok(typeof personal === 'object' && personal !== null)
    assert.ok(
      'rate_basis' in personal &&
        'user_price' in personal &&
        'models' in personal
    )
    assert.equal(personal.rate_basis, 'effective_user_rate')
    assert.deepEqual(personal.user_price, {
      status: 'verified',
      multiplier: { numerator: '97', denominator: '100' },
    })
    // The returned amount already contains the factor; metadata cannot reprice it.
    assert.deepEqual(personal.models, output.models)
  } finally {
    cleanup()
    api.defaults.adapter = adapter
    useAuthStore.setState({ auth: previousAuth })
    Object.defineProperty(globalThis, 'document', {
      configurable: true,
      value: previousDocument,
    })
  }
})
