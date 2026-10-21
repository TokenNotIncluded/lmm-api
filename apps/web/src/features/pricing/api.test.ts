/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { AxiosError } from 'axios'

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

function setSession(userId: number, sid: string) {
  useAuthStore.getState().auth.setBundle({
    access_token: `synthetic-token-${sid}`,
    token_type: 'Bearer',
    access_expires_at: 4_000_000_000,
    user: { id: userId, username: 'synthetic', role: 1 },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 4_000_000_000,
    },
  })
}

function deferred() {
  let finish: (() => void) | undefined
  const promise = new Promise<void>((resolve) => {
    finish = resolve
  })
  return {
    promise,
    resolve() {
      assert.ok(finish)
      finish()
    },
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

test('a query captured for an old account cannot dispatch using a newer account', async () => {
  setSession(8, 'new-session')
  let requests = 0
  api.defaults.adapter = async () => {
    requests++
    throw new Error('A stale query must not dispatch')
  }
  await assert.rejects(
    getPricing(undefined, { userId: 7, sessionId: 'old-session' }),
    /Authentication changed before request could be sent/
  )
  assert.equal(requests, 0)
})

for (const [name, nextUserId, nextSID] of [
  ['account switch', 8, 'new-session'],
  ['same-account session switch', 7, 'new-session'],
] as const) {
  test(`a late pricing response is rejected after ${name}`, async () => {
    setSession(7, 'old-session')
    const sent = deferred()
    const release = deferred()
    api.defaults.adapter = async (config) => {
      sent.resolve()
      await release.promise
      return {
        data: document('session'),
        status: 200,
        statusText: 'OK',
        headers: {},
        config,
      }
    }
    const result = getPricing(undefined, {
      userId: 7,
      sessionId: 'old-session',
    })
    const rejected = assert.rejects(
      result,
      /Authentication changed before request could be sent/
    )
    await sent.promise
    setSession(nextUserId, nextSID)
    release.resolve()
    await rejected
  })
}

test('a public response cannot become account pricing after sign-in', async () => {
  useAuthStore.getState().auth.reset('complete')
  const sent = deferred()
  const release = deferred()
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/core/v1/pricing/public')
    assert.equal(config.authMode, 'anonymous')
    sent.resolve()
    await release.promise
    return {
      data: document('public'),
      status: 200,
      statusText: 'OK',
      headers: {},
      config,
    }
  }
  const result = getPricing(undefined, {
    userId: undefined,
    sessionId: undefined,
  })
  const rejected = assert.rejects(
    result,
    /Authentication changed before request could be sent/
  )
  await sent.promise
  setSession(7, 'signed-in')
  release.resolve()
  await rejected
})

test('unavailable account pricing is an error without an anonymous fallback', async () => {
  setSession(7, 'session')
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    requests.push(config.url ?? '')
    throw new AxiosError(
      'Account pricing unavailable',
      undefined,
      config,
      undefined,
      {
        data: { error: 'user_price_unknown' },
        status: 503,
        statusText: 'Service Unavailable',
        headers: {},
        config,
      }
    )
  }
  await assert.rejects(getPricing(), (error: unknown) => {
    assert.ok(error instanceof AxiosError)
    assert.equal(error.response?.status, 503)
    return true
  })
  assert.deepEqual(requests, ['/core/v1/pricing/session'])
})
