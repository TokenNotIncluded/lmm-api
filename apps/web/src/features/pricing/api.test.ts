/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { getPricing, pricingQueryKey } from './api'
import type { NativePricingDocument } from './catalog-contract'

const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
})

function document(visibility: 'public' | 'session'): NativePricingDocument {
  return {
    schema_version: 1,
    visibility,
    observed_at: 100,
    credits_per_usd: 500_000,
    rate_basis: 'effective_group_rate',
    invocation_authorized: false,
    routing_version: 1,
    pricing_version: 2,
    image_pricing_version: null,
    groups: [],
    models: [],
  }
}

test('anonymous pricing uses the anonymous endpoint, timeout and real cancellation signal', async () => {
  useAuthStore.getState().auth.reset('complete')
  const signal = new AbortController().signal
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    assert.equal(config.url, '/core/v1/pricing/public')
    assert.equal(config.authMode, 'anonymous')
    assert.equal(config.timeout, 10_000)
    assert.equal(config.signal, signal)
    assert.equal(config.headers.get('Authorization'), undefined)
    return {
      data: document('public'),
      status: 200,
      statusText: 'OK',
      headers: {},
      config,
    }
  }
  assert.equal((await getPricing(signal)).data.length, 0)
  assert.equal(requests, 1)
})

test('Session pricing is distinct from OAuth and rejects a different visibility document', async () => {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-browser-session',
    token_type: 'Bearer',
    access_expires_at: 4_000_000_000,
    user: { id: 7, username: 'synthetic', role: 1 },
    session: {
      sid: 'synthetic-session',
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 4_000_000_000,
    },
  })
  let visibility: 'public' | 'session' = 'session'
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/core/v1/pricing/session')
    assert.equal(config.authMode, 'session')
    assert.equal(
      config.headers.get('Authorization'),
      'Bearer synthetic-browser-session'
    )
    return {
      data: document(visibility),
      status: 200,
      statusText: 'OK',
      headers: {},
      config,
    }
  }
  assert.equal((await getPricing()).success, true)
  visibility = 'public'
  await assert.rejects(getPricing(), /visibility did not match/)
  assert.notDeepEqual(pricingQueryKey(7, 'one'), pricingQueryKey(7, 'two'))
  assert.notDeepEqual(pricingQueryKey(7, 'one'), pricingQueryKey(8, 'one'))
  assert.notDeepEqual(pricingQueryKey(), pricingQueryKey(7, 'one'))
})

test('the removed success envelope and malformed documents are not successful catalogues', async () => {
  useAuthStore.getState().auth.reset('complete')
  api.defaults.adapter = async (config) => ({
    data: { success: true, data: [] },
    status: 200,
    statusText: 'OK',
    headers: {},
    config,
  })
  await assert.rejects(getPricing())
})
