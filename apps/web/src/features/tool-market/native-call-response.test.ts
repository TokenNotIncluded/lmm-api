/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { api } from '@/lib/api'

import { marketAPI } from './api'
import {
  MarketAPIError,
  marketConfirmationURL,
  parseMarketCallResponse,
} from './native-call-response'

const callID = 'a'.repeat(64)
const state = 'b'.repeat(64)
const imagePath = `/api/user/auth/images/confirm/image:${'d'.repeat(64)}`
const bountyPath = `/core/v1/bounties/confirm/${'e'.repeat(64)}`
function receipt() {
  return {
    call: {
      id: callID,
      user_id: 17,
      client_id: 'web-market',
      tool_id: 'builtin-tool',
      version_id: 'version',
      grant_id: 'grant',
      created_at: 10,
      resolve_by: 100,
      execution_status: 'awaiting_confirmation',
      settlement_status: 'reserved',
      price_quota: 0,
    },
    result: {
      content: [],
      structuredContent: {
        confirmation_url: imagePath,
      },
    },
    awaiting_confirmation: true,
    request_state: state,
    result_expired: false,
    error_code: 'TOOL_MARKET_EXECUTION_PENDING',
  }
}
function pending() {
  return {
    success: false,
    message: '',
    code: 'TOOL_MARKET_EXECUTION_PENDING',
    data: receipt(),
  }
}

test('409 pending retains the original call, opaque state and preview', () => {
  const result = parseMarketCallResponse(pending(), 409)
  assert.equal(result.call.id, callID)
  assert.equal(result.call.grant_id, 'grant')
  assert.equal(result.call.user_id, 17)
  assert.equal(result.request_state, state)
  assert.equal(result.awaiting_confirmation, true)
  assert.deepEqual(result.result, receipt().result)
})

test('a 200 terminal receipt retains the actual result and does not accept a pending envelope', () => {
  const result = {
    ...receipt(),
    call: {
      ...receipt().call,
      execution_status: 'succeeded',
      settlement_status: 'settled',
    },
    result: {
      content: [{ type: 'text', text: 'Complete' }],
      structuredContent: { output: { ok: true } },
    },
    awaiting_confirmation: false,
    error_code: undefined,
  }
  assert.deepEqual(
    parseMarketCallResponse({ success: true, message: '', data: result }, 200)
      .result,
    result.result
  )
  assert.throws(() =>
    parseMarketCallResponse(
      { success: true, message: '', data: receipt() },
      200
    )
  )
})

test('HTTP status, envelope and safe receipt values are checked together', () => {
  const invalid: unknown[] = [
    { ...pending(), success: true },
    { ...pending(), data: undefined },
    { ...pending(), data: { ...receipt(), awaiting_confirmation: false } },
    { ...pending(), data: { ...receipt(), request_state: 'invented' } },
    { ...pending(), data: { ...receipt(), extra: 'unreviewed authority' } },
    {
      ...pending(),
      data: {
        ...receipt(),
        call: { ...receipt().call, price_quota: Number.MAX_SAFE_INTEGER + 1 },
      },
    },
    {
      ...pending(),
      data: {
        ...receipt(),
        call: { ...receipt().call, settlement_status: 'invented' },
      },
    },
    { ...pending(), data: { ...receipt(), result: 'pretend completed' } },
    { ...pending(), data: { ...receipt(), result_expired: true } },
  ]
  for (const body of invalid) {
    assert.throws(
      () => parseMarketCallResponse(body, 409),
      (error) =>
        error instanceof MarketAPIError &&
        error.code === 'TOOL_MARKET_INVALID_RESPONSE'
    )
  }
  assert.throws(() => parseMarketCallResponse(pending(), 200))
  assert.throws(() => parseMarketCallResponse(pending(), 503))
  const { price_quota: _price, ...withoutPrice } = receipt().call
  assert.equal(
    parseMarketCallResponse(
      { ...pending(), data: { ...receipt(), call: withoutPrice } },
      409
    ).call.price_quota,
    undefined
  )
})

test('missing original requests are explicit errors, never empty successful receipts', () => {
  assert.throws(
    () =>
      parseMarketCallResponse(
        {
          success: false,
          code: 'TOOL_MARKET_NOT_FOUND',
          message: 'private server diagnostics',
        },
        404
      ),
    (error) =>
      error instanceof MarketAPIError &&
      error.code === 'TOOL_MARKET_NOT_FOUND' &&
      !error.message.includes('private')
  )
  assert.throws(
    () =>
      parseMarketCallResponse(
        { success: false, code: 'TOOL_MARKET_EXECUTION_PENDING', message: '' },
        409
      ),
    (error) =>
      error instanceof MarketAPIError &&
      error.code === 'TOOL_MARKET_INVALID_RESPONSE'
  )
})

test('confirmation links are same-origin exact Core confirmation paths', () => {
  const origin = 'https://console.example.test'
  assert.equal(
    marketConfirmationURL(receipt().result, origin),
    `${origin}${imagePath}`
  )
  assert.equal(
    marketConfirmationURL(
      { structuredContent: { confirmation_url: `${origin}${imagePath}` } },
      origin
    ),
    `${origin}${imagePath}`
  )
  assert.equal(
    marketConfirmationURL(
      {
        structuredContent: {
          receipt: { confirmation_url: bountyPath },
        },
      },
      origin
    ),
    `${origin}${bountyPath}`
  )
  for (const url of [
    `/core/v1/images/confirm/image:${'d'.repeat(64)}`,
    `${origin}/core/v1/images/confirm/image:${'d'.repeat(64)}`,
    '/api/user/auth/images/confirm/image_123',
    '/api/user/auth/images/confirm/image:short',
    `/api/user/auth/images/confirm/image:${'D'.repeat(64)}`,
    '/core/v1/bounties/confirm/not-a-confirmation-hash',
    `https://attacker.test${imagePath}`,
    'javascript:alert(1)',
    '/logout',
    '/api/user/auth/refresh',
    '/api/user/auth/logout',
    `${imagePath}/`,
    `${imagePath}?approve=true`,
    `${imagePath}#token`,
    `/api/user/auth/images/confirm/../confirm/image:${'d'.repeat(64)}`,
    `${origin}/api/user/auth/images/confirm/%2e%2e/confirm/image:${'d'.repeat(64)}`,
    `/api/user/auth/./images/confirm/image:${'d'.repeat(64)}`,
    `/api/user/auth/images/confirm/other/../image:${'d'.repeat(64)}`,
    `https://user@console.example.test${imagePath}`,
    `https://user:password@console.example.test${imagePath}`,
    `https://@console.example.test${imagePath}`,
    `https://:@console.example.test${imagePath}`,
    `https://console.example.test\t${imagePath}`,
    String.raw`https://console.example.test\junk\..${imagePath}`,
  ]) {
    assert.equal(
      marketConfirmationURL(
        { structuredContent: { confirmation_url: url } },
        origin
      ),
      undefined
    )
  }
})

test('shared HTTP methods keep pending data and only resume uses POST', async () => {
  const oldAdapter = api.defaults.adapter
  const requests: { method?: string; url?: string; body: unknown }[] = []
  api.defaults.adapter = async (config) => {
    requests.push({ method: config.method, url: config.url, body: config.data })
    assert.equal(config.skipBusinessError, true)
    assert.equal(config.skipErrorHandler, true)
    return {
      status: 409,
      statusText: 'Conflict',
      headers: {},
      config,
      data: pending(),
    }
  }
  try {
    const input = {
      tool_id: 'builtin-tool',
      version_id: 'version',
      grant_id: 'grant',
      request_id: 'original-request',
      arguments: {},
    }
    assert.equal((await marketAPI.invoke(input)).request_state, state)
    assert.equal((await marketAPI.result(callID)).call.id, callID)
    assert.equal(
      (await marketAPI.lookupRequest(input.request_id)).call.id,
      callID
    )
    assert.equal(
      (await marketAPI.resume(callID, { request_state: state })).call.id,
      callID
    )
    assert.deepEqual(
      requests.map((item) => item.method),
      ['post', 'get', 'get', 'post']
    )
    assert.equal(
      requests[2].url,
      '/api/tool-market/executions/by-request/original-request'
    )
    assert.equal(requests[3].url, `/api/tool-market/calls/${callID}/resume`)
    assert.deepEqual(JSON.parse(String(requests[3].body)), {
      request_state: state,
    })
    assert.equal(requests.length, 4)
  } finally {
    api.defaults.adapter = oldAdapter
  }
})
