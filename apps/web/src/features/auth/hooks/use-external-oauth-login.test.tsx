/*
Copyright (C) 2026 LIghtJUNction

SPDX-License-Identifier: AGPL-3.0-or-later
*/
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

import {
  EXTERNAL_BROWSER_PRESETS,
  type ExternalBrowserProvider,
} from '../lib/external-oauth'
import type { SystemStatus } from '../types'

const dom = new Window({ url: 'https://site.example.test/sign-in' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const calls: { path: string; payload: unknown; options: unknown }[] = []
const legacyFlows: { provider: string; mode: string; legal: boolean }[] = []
const navigations: string[] = []
const errors: string[] = []
let logoutCalls = 0
let response: unknown
let pendingResponse: Promise<unknown> | undefined
Object.defineProperty(dom.location, 'assign', {
  configurable: true,
  value: (url: string) => navigations.push(url),
})
const t = (key: string) => key
mock.module('react-i18next', () => ({ useTranslation: () => ({ t }) }))
mock.module('sonner', () => ({
  toast: { error: (value: string) => errors.push(value), success: () => {} },
}))
mock.module('@/lib/api', () => ({
  api: {
    post: async (path: string, payload: unknown, options: unknown) => {
      calls.push({ path, payload, options })
      return { data: pendingResponse ? await pendingResponse : response }
    },
  },
  isAuthBundle: () => false,
}))
mock.module('../api', () => ({
  createOAuthFlow: async (provider: string, mode: string, legal: boolean) => {
    legacyFlows.push({ provider, mode, legal })
    return 'legacy-flow-state'
  },
  logout: async () => {
    logoutCalls++
    return { success: true }
  },
  startTelegramLogin: async () => '',
  telegramLogin: async () => ({ success: false }),
}))
mock.module('./use-auth-redirect', () => ({
  useAuthRedirect: () => ({ handleLoginSuccess: async () => {} }),
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { useOAuthLogin } = await import('./use-oauth-login')
let root: ReturnType<typeof createRoot> | undefined
let login: () => Promise<void>
function Harness({
  status,
  provider,
  legal,
}: {
  status: SystemStatus
  provider: ExternalBrowserProvider
  legal: boolean
}) {
  const flow = useOAuthLogin(status, '/console', legal)
  switch (provider) {
    case 'github':
      login = flow.handleGitHubLogin
      break
    case 'discord':
      login = flow.handleDiscordLogin
      break
    case 'linuxdo':
      login = flow.handleLinuxDOLogin
      break
  }
  return null
}
const providers: ExternalBrowserProvider[] = ['github', 'discord', 'linuxdo']
function configured(provider: ExternalBrowserProvider): SystemStatus {
  return {
    external_oauth_browser: true,
    [`${provider}_oauth`]: true,
    [`${provider}_client_id`]: 'synthetic-client',
  }
}
function validResponse(provider: ExternalBrowserProvider) {
  const preset = EXTERNAL_BROWSER_PRESETS[provider]
  const url = new URL(preset.authorization_endpoint)
  for (const [key, value] of Object.entries({
    client_id: 'synthetic-client',
    response_type: 'code',
    scope: preset.scope,
    state: 'server-generated-state',
    redirect_uri: `https://site.example.test${preset.callbacks[0]}`,
    provider_option: 'preserved-value',
  })) {
    url.searchParams.set(key, value)
  }
  if (preset.pkce) {
    url.searchParams.set('code_challenge', 'server-generated-proof')
    url.searchParams.set('code_challenge_method', 'S256')
  }
  return { success: true, data: { authorization_url: url.toString() } }
}
async function mount(
  provider: ExternalBrowserProvider,
  status = configured(provider),
  legal = false
) {
  const host = document.createElement('div')
  document.body.append(host)
  const currentRoot = createRoot(host)
  root = currentRoot
  await act(async () =>
    currentRoot.render(
      <Harness status={status} provider={provider} legal={legal} />
    )
  )
}
afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = undefined
  document.body.replaceChildren()
  calls.length = legacyFlows.length = navigations.length = errors.length = 0
  logoutCalls = 0
  pendingResponse = undefined
})
after(() => {
  mock.restore()
  dom.close()
})

describe('built-in external browser hook contract', () => {
  for (const provider of providers) {
    test(`${provider} starts the browser-owned flow and preserves its URL without legacy session reset`, async () => {
      response = validResponse(provider)
      await mount(provider, configured(provider), true)
      await act(async () => login())
      assert.deepEqual(calls, [
        {
          path: `/api/oauth/${provider}/start`,
          payload: { accepted_legal: true },
          options: { skipAuthRefresh: true },
        },
      ])
      assert.deepEqual(navigations, [
        validResponse(provider).data.authorization_url,
      ])
      assert.equal(logoutCalls, 0)
      assert.deepEqual(legacyFlows, [])
      assert.deepEqual(errors, [])
    })

    test(`${provider} preserves false consent and refuses required legal consent before the start`, async () => {
      response = validResponse(provider)
      await mount(provider, {
        ...configured(provider),
        privacy_policy_enabled: true,
      })
      await act(async () => login())
      assert.deepEqual(calls, [])
      assert.deepEqual(navigations, [])
      assert.equal(logoutCalls, 0)
      assert.equal(errors.length, 1)
    })

    test(`${provider} never invents legal consent when the site does not require it`, async () => {
      response = validResponse(provider)
      await mount(provider)
      await act(async () => login())
      assert.deepEqual(
        calls.map((call) => call.payload),
        [{ accepted_legal: false }]
      )
      assert.equal(navigations.length, 1)
    })

    test(`${provider} keeps the old flow only when browser capability is not advertised`, async () => {
      await mount(
        provider,
        { ...configured(provider), external_oauth_browser: false },
        true
      )
      await act(async () => login())
      assert.deepEqual(calls, [])
      assert.equal(logoutCalls, 1)
      assert.deepEqual(legacyFlows, [{ provider, mode: 'login', legal: true }])
      assert.equal(navigations.length, 1)
      assert.equal(
        new URL(navigations[0]).searchParams.get('state'),
        'legacy-flow-state'
      )
    })

    test(`${provider} rejects a malformed native-token or foreign response`, async () => {
      await mount(provider)
      for (const malformedResponse of [
        { success: true, data: { access_token: 'not-a-browser-flow' } },
        {
          success: true,
          data: { authorization_url: 'https://foreign.example.test' },
        },
      ]) {
        response = malformedResponse
        await act(async () => login())
      }
      assert.equal(calls.length, 2)
      assert.equal(errors.length, 2)
      assert.deepEqual(navigations, [])
      assert.equal(logoutCalls, 0)
      assert.deepEqual(legacyFlows, [])
    })

    test(`${provider} synchronously ignores double starts while its cookie-binding request is pending`, async () => {
      let resolve!: (value: unknown) => void
      pendingResponse = new Promise((done) => {
        resolve = done
      })
      await mount(provider)
      let first!: Promise<void>
      await act(async () => {
        first = login()
        await login()
      })
      assert.equal(calls.length, 1)
      assert.deepEqual(navigations, [])
      await act(async () => {
        resolve(validResponse(provider))
        await first
      })
      assert.equal(calls.length, 1)
      assert.equal(navigations.length, 1)
    })
  }
})
