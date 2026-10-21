/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import axios, { AxiosError } from 'axios'
import { toast } from 'sonner'

import { api } from '@/lib/api'
import { AUTH_COOKIE_TIMEOUT_MS } from '@/lib/auth-cookie'
import {
  clearAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

import { createOAuthFlow, executeLogout, login, startWechatLogin } from './api'

describe('password login request errors', () => {
  test('keeps Cookie and session semantics while leaving HTTP and business notifications to the form', async () => {
    const originalAdapter = api.defaults.adapter
    const originalToast = toast.error
    const notifications: unknown[] = []
    let refreshes = 0
    let requests = 0
    const account: AuthBundle = {
      access_token: 'synthetic-existing-token',
      token_type: 'Bearer',
      access_expires_at: Math.floor(Date.now() / 1000) + 600,
      user: { id: 42, username: 'synthetic', role: 1 },
      session: {
        sid: 'synthetic-existing-session',
        current: true,
        login_method: 'password',
        ip: '',
        user_agent: '',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2_000_000_000,
      },
    }
    clearAuthentication(false, 'idle')
    useAuthStore.getState().auth.setBundle(account)
    toast.error = (message) => {
      notifications.push(message)
      return 'password-login-test'
    }
    setDevelopmentAuthRefreshAdapter(async () => {
      refreshes++
      throw new Error('Password login must not refresh or replay')
    })

    try {
      for (const status of [401, 403, 429, 503, 200]) {
        const payload = {
          success: false,
          code: status === 401 ? 'AUTH_UNAUTHORIZED' : 'UNKNOWN_FAILURE',
          message: 'private backend diagnostic must not appear in a toast',
        }
        api.defaults.adapter = async (config) => {
          requests++
          assert.equal(config.method, 'post')
          assert.equal(config.url, '/api/user/login?turnstile=synthetic-proof')
          assert.deepEqual(JSON.parse(config.data), {
            username: 'synthetic',
            password: 'synthetic-password',
          })
          assert.equal(config.withCredentials, true)
          assert.equal(config.authMode, undefined)
          assert.equal(
            config.headers.get('Authorization'),
            'Bearer synthetic-existing-token'
          )
          assert.equal(config.skipAuthRefresh, true)
          assert.equal(config.skipErrorHandler, true)
          assert.equal(config.skipBusinessError, true)
          assert.equal(config.timeout, AUTH_COOKIE_TIMEOUT_MS)
          assert.equal(
            config.signal,
            undefined,
            'an active Cookie writer must not be aborted by the form'
          )
          const response = {
            config,
            status,
            statusText: 'synthetic response',
            headers: {},
            data: payload,
          }
          if (status >= 400) {
            throw new AxiosError(
              'private transport diagnostic',
              'ERR_BAD_RESPONSE',
              config,
              undefined,
              response
            )
          }
          return response
        }
        const attempt = login(
          {
            username: 'synthetic',
            password: 'synthetic-password',
            turnstile: 'synthetic-proof',
          },
          new AbortController().signal
        )
        if (status === 200) {
          assert.deepEqual(await attempt, payload)
        } else {
          await assert.rejects(
            attempt,
            (error: unknown) =>
              error instanceof AxiosError && error.response?.status === status
          )
        }
        assert.deepEqual(notifications, [])
        assert.equal(
          useAuthStore.getState().auth.accessToken,
          account.access_token
        )
        assert.equal(
          useAuthStore.getState().auth.session?.sid,
          account.session.sid
        )
      }
      assert.equal(
        requests,
        5,
        'failures must never replay the password request'
      )
      assert.equal(refreshes, 0)
    } finally {
      api.defaults.adapter = originalAdapter
      setDevelopmentAuthRefreshAdapter(axios.getAdapter(originalAdapter))
      toast.error = originalToast
      clearAuthentication(false, 'idle')
    }
  })

  test('an incomplete MFA response keeps an allowlisted error code', async () => {
    const originalAdapter = api.defaults.adapter
    clearAuthentication(false, 'idle')
    api.defaults.adapter = async (config) => ({
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: { require_2fa: true } },
    })
    try {
      await assert.rejects(
        login({ username: 'synthetic', password: 'synthetic-password' }),
        (error: unknown) =>
          error instanceof Error &&
          'code' in error &&
          error.code === 'MFA_FLOW_EXPIRED'
      )
      assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, null)
      assert.equal(useAuthStore.getState().auth.user, null)
    } finally {
      api.defaults.adapter = originalAdapter
      clearAuthentication(false, 'idle')
    }
  })
})

function mismatchError() {
  return {
    isAxiosError: true,
    response: {
      status: 409,
      data: { code: 'AUTH_SESSION_MISMATCH' },
    },
  }
}

describe('logout coordination', () => {
  test('returns an unsuccessful response without pretending to sign out', async () => {
    assert.deepEqual(
      await executeLogout({
        getExpectedSID: () => 'session-a',
        request: async () => ({ success: false, message: 'not revoked' }),
      }),
      { success: false, message: 'not revoked' }
    )
  })

  test('a cookie mismatch is rejected without hidden refresh or logout replay', async () => {
    const requestedSIDs: Array<string | undefined> = []
    const error = mismatchError()
    await assert.rejects(
      executeLogout({
        getExpectedSID: () => 'session-a',
        request: async (sid) => {
          requestedSIDs.push(sid)
          throw error
        },
      }),
      (actual) => actual === error
    )
    assert.deepEqual(requestedSIDs, ['session-a'])
  })

  test('treats an already anonymous or legacy 404 logout endpoint as signed out', async () => {
    for (const status of [401, 404]) {
      const result = await executeLogout({
        getExpectedSID: () => 'session-a',
        request: async () => {
          throw {
            isAxiosError: true,
            response: { status, data: { code: 'AUTH_NOT_FOUND' } },
          }
        },
      })
      assert.deepEqual(result, { success: true, message: '' })
    }
  })
})

describe('OAuth flow initialization', () => {
  test('carries the stored invitation into the browser-bound WeChat flow', async () => {
    const originalPost = api.post
    const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
    const requests: Array<{ body: unknown; config: unknown }> = []
    let affiliateCode: string | null = 'inviter-code'
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        localStorage: {
          getItem: (key: string) => {
            assert.equal(key, 'aff')
            return affiliateCode
          },
        },
      },
    })
    api.post = (async (url: string, body: unknown, config: unknown) => {
      assert.equal(url, '/api/oauth/wechat/start')
      requests.push({ body, config })
      return {
        data: { success: true, data: { flow_token: 'wechat-flow-token' } },
      }
    }) as typeof api.post

    try {
      assert.equal(await startWechatLogin(true), 'wechat-flow-token')
      affiliateCode = null
      await startWechatLogin()
      assert.deepEqual(requests, [
        {
          body: { accepted_legal: true, aff: 'inviter-code' },
          config: { skipAuthRefresh: true },
        },
        {
          body: { accepted_legal: false, aff: undefined },
          config: { skipAuthRefresh: true },
        },
      ])
    } finally {
      api.post = originalPost
      if (originalWindow) {
        Object.defineProperty(globalThis, 'window', originalWindow)
      } else {
        Reflect.deleteProperty(globalThis, 'window')
      }
    }
  })

  test('sends legal consent for login flows', async () => {
    const originalPost = api.post
    let requestBody: unknown
    api.post = (async (url: string, body: unknown) => {
      assert.equal(url, '/api/oauth/state')
      requestBody = body
      return {
        data: {
          success: true,
          data: { flow_token: 'oauth-flow-token' },
        },
      }
    }) as typeof api.post

    try {
      const token = await createOAuthFlow('linuxdo', 'login', true)
      assert.equal(token, 'oauth-flow-token')
      assert.deepEqual(requestBody, {
        provider: 'linuxdo',
        intent: 'login',
        aff: undefined,
        accepted_legal: true,
      })
    } finally {
      api.post = originalPost
    }
  })

  test('does not attach registration consent to binding flows', async () => {
    const originalPost = api.post
    let requestBody: unknown
    api.post = (async (_url: string, body: unknown) => {
      requestBody = body
      return {
        data: {
          success: true,
          data: { flow_token: 'oauth-bind-token' },
        },
      }
    }) as typeof api.post

    try {
      await createOAuthFlow('github', 'bind', true)
      assert.deepEqual(requestBody, {
        provider: 'github',
        intent: 'bind',
        aff: undefined,
        accepted_legal: undefined,
      })
    } finally {
      api.post = originalPost
    }
  })
})
