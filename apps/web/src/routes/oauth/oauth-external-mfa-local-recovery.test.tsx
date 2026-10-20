/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import type { AxiosAdapter, AxiosResponse } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/oauth/google' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'MutationObserver',
  'getComputedStyle',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const navigation: Array<{ to?: string; href?: string }> = []
const navigate = (value: { to?: string; href?: string }) => {
  navigation.push(value)
}
const search = { external_mfa: '1' }
const provider = 'google'
const realRouter = await import('@tanstack/react-router')
mock.module('@tanstack/react-router', () => ({
  ...realRouter,
  useNavigate: () => navigate,
  useParams: () => ({ provider }),
  useSearch: () => search,
}))
// Replace only presentation and feedback sinks; the real OAuth effect, HTTP
// adapter, redirect hook, MFA commit and auth generations remain exercised.
mock.module('@/features/auth/components/oauth-callback-screen', () => ({
  OAuthCallbackScreen: () => <div>Callback</div>,
}))
mock.module('sonner', () => ({
  toast: {
    error: () => undefined,
    success: () => undefined,
    info: () => undefined,
  },
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { Route } = await import('./$provider')
const { api } = await import('@/lib/api')
const {
  applyAuthBundle,
  captureLocalAuthRecoveryScope,
  endLocalAuthSessionForRecovery,
} = await import('@/lib/auth-session')
const { useAuthStore } = await import('@/stores/auth-store')
const originalAdapter = api.defaults.adapter
const Callback = Route.options.component
if (!Callback) throw new Error('OAuth callback component is not registered')
// This file gets a fresh test process: an uncertain-cookie document stays
// blocked even after storage is cleared, so recovery cases cannot reset it.
const bundle = {
  access_token: 'synthetic-local-state',
  token_type: 'Bearer',
  access_expires_at: 2_000_000_000,
  user: { id: 42, username: 'synthetic', role: 1 },
  session: {
    sid: 'synthetic-sid',
    current: true,
    login_method: 'password',
    ip: '',
    user_agent: '',
    created_at: 1,
    last_active_at: 1,
    expires_at: 2_000_000_000,
  },
}
let dispatches = 0
function deferredCallback() {
  let entered!: () => void
  const started = new Promise<void>((resolve) => {
    entered = resolve
  })
  let resolve!: (response: AxiosResponse) => void
  let requestConfig!: Parameters<AxiosAdapter>[0]
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, `/api/oauth/${provider}`)
    if (search.external_mfa === '1') {
      assert.deepEqual(config.params, { external_mfa: '1' })
    }
    dispatches++
    requestConfig = config
    entered()
    return new Promise<AxiosResponse>((done) => {
      resolve = done
    })
  }
  return {
    started,
    finish(flow: string) {
      resolve({
        config: requestConfig,
        status: 200,
        statusText: 'synthetic',
        headers: {},
        data: {
          success: true,
          data: {
            require_2fa: true,
            flow_token: flow,
            expires_at: 2_000_000_000,
          },
        },
      })
    },
  }
}
after(() => {
  api.defaults.adapter = originalAdapter
  mock.restore()
  dom.close()
})

test('healthy cookie-only external MFA continuation supersedes an earlier same-account local recovery control', async () => {
  applyAuthBundle(bundle, false)
  const before = captureLocalAuthRecoveryScope()
  const task = deferredCallback()
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () => root.render(<Callback />))
    await task.started
    await act(async () => {
      task.finish('synthetic-google-cookie-mfa')
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    assert.equal(
      useAuthStore.getState().auth.pending2FAFlowToken,
      'synthetic-google-cookie-mfa'
    )
    const current = captureLocalAuthRecoveryScope()
    assert.equal(current.accountWriteEpoch, before.accountWriteEpoch)
    assert.equal(current.authEpoch, before.authEpoch + 1)
    assert.equal(endLocalAuthSessionForRecovery(before).kind, 'superseded')
  } finally {
    await act(async () => root.unmount())
  }
})

test('a late cookie-only external MFA response cannot restore login after explicit local recovery', async () => {
  applyAuthBundle(bundle, false)
  const task = deferredCallback()
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const beforeOtp = navigation.filter((entry) => entry.to === '/otp').length
  try {
    await act(async () => root.render(<Callback />))
    await task.started
    window.localStorage.setItem(
      'new-api:auth-cookie:uncertain',
      'uncertain:synthetic-late-cookie-mfa'
    )
    assert.equal(
      endLocalAuthSessionForRecovery(captureLocalAuthRecoveryScope()).kind,
      'ended'
    )
    await act(async () => {
      task.finish('synthetic-late-cookie-mfa')
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, null)
    assert.equal(useAuthStore.getState().auth.user, null)
    assert.equal(
      navigation.filter((entry) => entry.to === '/otp').length,
      beforeOtp
    )
    assert.equal(
      dispatches,
      2,
      'one original GET for each callback; no refresh or replay'
    )
  } finally {
    await act(async () => root.unmount())
  }
})
