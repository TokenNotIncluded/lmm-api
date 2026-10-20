/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { z } from 'zod'

import { api } from '@/lib/api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import type { WebMcpRouter } from '../tool-kit'
import { accountTools } from './account'

const originalGet = api.get
const originalAuth = useAuthStore.getState().auth
const calls: Array<{ to: string }> = []
const router: WebMcpRouter = {
  navigate: async (options) => {
    calls.push(options)
  },
  subscribe: () => () => undefined,
}
const tools = accountTools({ router })
const options = () => ({ signal: new AbortController().signal })
const find = (name: string) => {
  const tool = tools.find((entry) => entry.name === name)
  assert.ok(tool)
  return tool
}
function signIn(role: number = ROLE.USER) {
  useAuthStore.setState({
    auth: { ...originalAuth, user: { id: 1, role } } as never,
  })
}
afterEach(() => {
  api.get = originalGet
  useAuthStore.setState({ auth: originalAuth })
  calls.length = 0
})

test('all account tools require sign-in and expose no mutations', async () => {
  useAuthStore.setState({ auth: { ...originalAuth, user: null } })
  assert.equal(new Set(tools.map((tool) => tool.name)).size, tools.length)
  for (const tool of tools) {
    assert.match(tool.name, /^lmm_account_[a-z0-9_]+$/)
    assert.equal(tool.annotations?.readOnlyHint, true)
    await assert.rejects(tool.execute({}, options()), /Sign in first/)
  }
  assert.equal(calls.length, 0)
})

test('subscription administration and root reset navigation enforce roles', async () => {
  signIn()
  for (const name of [
    'lmm_account_open_subscriptions',
    'lmm_account_open_subscription_reset',
    'lmm_account_list_subscription_plans',
  ]) {
    await assert.rejects(
      find(name).execute({}, options()),
      /administrator account/
    )
  }
  signIn(ROLE.ADMIN)
  await assert.rejects(
    find('lmm_account_open_subscription_reset').execute({}, options()),
    /root administrator/
  )
  signIn(ROLE.SUPER_ADMIN)
  await find('lmm_account_open_subscription_reset').execute({}, options())
  assert.deepEqual(calls, [{ to: '/subscriptions/reset' }])
})

test('profile and share responses never expose credentials or share tokens', async () => {
  signIn()
  api.get = (async () => ({
    data: {
      success: true,
      data: {
        id: 1,
        username: 'Alice',
        enabled: true,
        quota: 200,
        access_token: 'secret-access',
        password: 'secret-password',
        token: 'secret-share',
        url: 'https://example.test/secret-share.svg',
        setting: '{"webhook_secret":"secret-webhook"}',
      },
    },
  })) as typeof api.get
  const profile = await find('lmm_account_get_profile').execute({}, options())
  const share = await find('lmm_account_get_share_status').execute(
    {},
    options()
  )
  assert.equal((profile as { quota: number }).quota, 200)
  assert.deepEqual(share, { enabled: true, path: '/profile/share' })
  assert.doesNotMatch(
    JSON.stringify([profile, share]),
    /secret-|password|access_token|webhook/
  )
})

test('log tools only request self endpoints and strip raw content', async () => {
  signIn()
  const paths: string[] = []
  api.get = (async (path: string) => {
    paths.push(path)
    return {
      data: {
        success: true,
        data: {
          total: 1,
          page: 1,
          page_size: 20,
          items: [
            {
              id: 4,
              user_id: 1,
              created_at: 1,
              type: 2,
              model_name: 'test-model',
              prompt_tokens: 12,
              completion_tokens: 4,
              quota: 3,
              other: 'secret-detail',
              content: 'secret-prompt',
              key: 'secret-key',
              token_name: 'secret-token',
            },
          ],
        },
      },
    }
  }) as typeof api.get
  for (const category of ['common']) {
    const data = await find('lmm_account_list_usage_logs').execute(
      { category },
      options()
    )
    assert.doesNotMatch(JSON.stringify(data), /secret-/)
  }
  assert.equal(paths.length, 1)
  assert.match(paths[0], /^\/api\/log\/self\?p=1&page_size=20$/)
})

test('unsupported drawing/task history remains explicit and never becomes an empty result', async () => {
  signIn()
  const paths: string[] = []
  api.get = (async (path: string) => {
    paths.push(path)
    return {
      data: {
        success: false,
        code: 'USAGE_HISTORY_UNAVAILABLE',
        availability: 'unavailable',
        category: path.startsWith('/api/mj/') ? 'drawing' : 'task',
      },
    }
  }) as typeof api.get
  for (const category of ['drawing', 'task']) {
    const data = await find('lmm_account_list_usage_logs').execute(
      { category },
      options()
    )
    assert.deepEqual(data, {
      availability: 'unavailable',
      category,
      path: `/usage-logs/${category}`,
    })
    assert.equal(Object.hasOwn(data, 'items'), false)
    assert.equal(Object.hasOwn(data, 'total'), false)
  }
  assert.match(paths[0], /^\/api\/mj\/self\?/)
  assert.match(paths[1], /^\/api\/task\/self\?/)
})

test('usage tools preserve incomplete settlement versus true zero and raw legacy units', async () => {
  signIn()
  api.get = (async () => ({
    data: {
      success: true,
      data: {
        total: 3,
        page: 1,
        page_size: 20,
        items: [
          {
            id: 1,
            user_id: 1,
            created_at: 1,
            type: 5,
            content: 'No final usage receipt',
            quota: 0,
            state: 'reconcile',
            billing_basis: 'incomplete',
            quota_money_available: false,
            other: JSON.stringify({
              billing_basis: 'incomplete',
              request_state: 'reconcile',
              usage_status: 'incomplete',
              key: 'secret-key',
            }),
          },
          {
            id: 2,
            user_id: 1,
            created_at: 1,
            type: 2,
            content: 'Completed',
            quota: 0,
            state: 'settled',
            billing_basis: 'net_consumption',
            quota_money_available: true,
            quota_money_basis: 'captured_credits_per_usd',
            quota_usd: '0',
            other: JSON.stringify({
              billing_basis: 'net_consumption',
              request_state: 'settled',
              usage_status: 'settled',
            }),
          },
          {
            id: 3,
            user_id: 1,
            created_at: 1,
            type: 5,
            content: 'Historical record',
            quota: -12,
            quota_money_available: false,
            quota_money_basis: 'missing_credit_basis',
          },
        ],
      },
    },
  })) as typeof api.get
  const result = z
    .object({
      items: z.array(
        z.object({
          quota: z.number().nullable(),
          quota_available: z.boolean(),
          quota_money_available: z.boolean().nullable(),
          quota_money_basis: z.string().nullable(),
          billing_basis: z.string().nullable(),
          request_state: z.string().nullable(),
          usage_status: z.string().nullable(),
        })
      ),
    })
    .parse(await find('lmm_account_list_usage_logs').execute({}, options()))
  assert.deepEqual(result.items[0], {
    quota: null,
    quota_available: false,
    quota_money_available: false,
    quota_money_basis: null,
    billing_basis: 'incomplete',
    request_state: 'reconcile',
    usage_status: 'incomplete',
  })
  assert.deepEqual(result.items[1], {
    quota: 0,
    quota_available: true,
    quota_money_available: true,
    quota_money_basis: 'captured_credits_per_usd',
    billing_basis: 'net_consumption',
    request_state: 'settled',
    usage_status: 'settled',
  })
  assert.equal(result.items[2].quota, -12)
  assert.equal(result.items[2].quota_available, true)
  assert.equal(result.items[2].quota_money_available, false)
  assert.doesNotMatch(JSON.stringify(result), /secret-key/)
})

test('business failure and malformed history are errors without exposing raw server content', async () => {
  signIn()
  for (const payload of [
    { success: false, message: 'secret-prompt' },
    { success: true },
    { success: true, data: { items: [], total: 0 } },
  ]) {
    api.get = (async () => ({ data: payload })) as typeof api.get
    await assert.rejects(
      find('lmm_account_list_usage_logs').execute({}, options()),
      (error: unknown) => {
        assert.ok(error instanceof Error)
        assert.doesNotMatch(error.message, /secret-prompt/)
        return true
      }
    )
  }
})

test('usage tools keep image counts, nullable money, actual holds and sponsorship without raw metadata', async () => {
  signIn()
  const base = {
    id: 1,
    user_id: 1,
    created_at: 1,
    content: 'redacted',
    pricing_dimension: 'image',
    token_usage_available: false,
    wallet_debit_supported: false,
  }
  const originalAdapter = api.defaults.adapter
  api.defaults.adapter = async (config) => ({
    config,
    status: 200,
    statusText: 'OK',
    headers: {},
    data: {
      success: true,
      data: {
        total: 2,
        page: 1,
        page_size: 20,
        items: [
          {
            ...base,
            type: 2,
            quota: 2000,
            output_image_count: 2,
            platform_sponsored: true,
            state: 'settled',
            other: JSON.stringify({
              credential_source: 'drawing_session',
              reserved_credits: 0,
              raw_prompt: 'secret-prompt',
            }),
          },
          {
            ...base,
            id: 2,
            type: 5,
            quota: null,
            quota_money_available: false,
            output_image_count: null,
            state: 'reconcile',
            billing_basis: 'incomplete',
            other: JSON.stringify({
              reserved_credits: 4000,
              raw_input: 'secret-input',
            }),
          },
        ],
      },
    },
  })
  try {
    const result = z
      .object({
        items: z.array(
          z.object({
            quota: z.number().nullable(),
            quota_available: z.boolean(),
            prompt_tokens: z.null(),
            completion_tokens: z.null(),
            pricing_dimension: z.literal('image'),
            token_usage_available: z.literal(false),
            output_image_count: z.number().nullable(),
            reserved_credits: z.number(),
            platform_sponsored: z.boolean().nullable(),
            wallet_debit_supported: z.literal(false),
          })
        ),
      })
      .parse(await find('lmm_account_list_usage_logs').execute({}, options()))
    assert.equal(result.items[0].output_image_count, 2)
    assert.equal(result.items[0].quota, 2000)
    assert.equal(result.items[0].platform_sponsored, true)
    assert.equal(result.items[1].quota, null)
    assert.equal(result.items[1].quota_available, false)
    assert.equal(result.items[1].output_image_count, null)
    assert.equal(result.items[1].reserved_credits, 4000)
    assert.doesNotMatch(
      JSON.stringify(result),
      /secret-|credential_source|raw_input|raw_prompt/
    )
  } finally {
    api.defaults.adapter = originalAdapter
  }
})

test('account tools discard a response after role or session identity changes', async () => {
  for (const change of ['role', 'session']) {
    signIn()
    api.get = (async () => {
      if (change === 'role') {
        signIn(ROLE.ADMIN)
      } else {
        useAuthStore.setState((state) => ({
          auth: {
            ...state.auth,
            session: {
              sid: 'changed-session',
              current: true,
              login_method: 'password',
              ip: null,
              user_agent: null,
              created_at: 1,
              last_active_at: 1,
              expires_at: 2,
            },
          },
        }))
      }
      return {
        data: { success: true, data: { id: 1, username: 'Late account' } },
      }
    }) as typeof api.get
    await assert.rejects(
      find('lmm_account_get_profile').execute({}, options()),
      /account changed/
    )
  }
})

test('per-model usage includes every bounded window without overlap', async () => {
  signIn()
  const ranges: Array<{ start_timestamp: number; end_timestamp: number }> = []
  api.get = (async (
    _path: string,
    config: { params: { start_timestamp: number; end_timestamp: number } }
  ) => {
    ranges.push(config.params)
    return {
      data: {
        success: true,
        data: [
          {
            model_name: 'free',
            created_at: config.params.start_timestamp,
            token_used: 100,
            count: 2,
            quota: 0,
          },
          {
            model_name: 'paid',
            created_at: config.params.start_timestamp,
            token_used: 10,
            count: 1,
            quota: 5,
          },
        ],
      },
    }
  }) as typeof api.get
  const result = (await find('lmm_account_get_model_usage').execute(
    { range: '365d' },
    options()
  )) as {
    totals: { tokens: number; requests: number }
    models: Array<{ modelName: string; share: number }>
  }
  assert.ok(ranges.length > 1)
  for (let i = 0; i < ranges.length; i += 1) {
    assert.ok(ranges[i].end_timestamp - ranges[i].start_timestamp <= 28 * 86400)
    if (i > 0) {
      assert.equal(ranges[i].start_timestamp, ranges[i - 1].end_timestamp + 1)
    }
  }
  assert.equal(result.totals.tokens, ranges.length * 110)
  assert.equal(result.totals.requests, ranges.length * 3)
  assert.equal(result.models.find((row) => row.modelName === 'free')?.share, 0)
  assert.equal(result.models.find((row) => row.modelName === 'paid')?.share, 1)
})

test('invalid input and aborted requests never access the API', async () => {
  signIn()
  let requested = false
  api.get = (async () => {
    requested = true
    throw new Error('unexpected')
  }) as typeof api.get
  await assert.rejects(
    find('lmm_account_list_usage_logs').execute(
      { page_size: 10000 },
      options()
    ),
    /page_size/
  )
  await assert.rejects(
    find('lmm_account_get_model_usage').execute({ range: 'all' }, options()),
    /range/
  )
  const controller = new AbortController()
  controller.abort()
  await assert.rejects(
    find('lmm_account_get_profile').execute({}, { signal: controller.signal })
  )
  assert.equal(requested, false)
})
