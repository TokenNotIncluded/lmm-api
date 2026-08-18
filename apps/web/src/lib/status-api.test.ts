/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { afterEach, describe, test } from 'node:test'

import { useAuthStore } from '@/stores/auth-store'

import { getStatus, extractStatusData } from './api'
import { setDevelopmentAuthRefreshAdapter } from './auth-session'
import { api } from './http-client'

const originalAdapter = api.defaults.adapter

afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.getState().auth.reset('idle')
})

describe('public status capability payload', () => {
  test('accepts a structured capability payload', () => {
    assert.deepEqual(extractStatusData({ data: { register_enabled: true } }), {
      register_enabled: true,
    })
  })

  test('fails instead of leaving registration loading forever when data is absent', () => {
    assert.throws(
      () => extractStatusData({ success: false, message: 'not ready' }),
      /Status response did not include capability data/
    )
    assert.throws(
      () => extractStatusData(null),
      /Status response did not include capability data/
    )
  })

  test('does not refresh an expired session while probing public capabilities', async () => {
    const now = Math.floor(Date.now() / 1000)
    useAuthStore.getState().auth.setBundle({
      access_token: 'expired-token',
      token_type: 'Bearer',
      access_expires_at: now - 1,
      user: { id: 42, username: 'stale', role: 1 },
      session: {
        sid: 'stale-session',
        current: true,
        login_method: 'password',
        ip: '127.0.0.1',
        user_agent: 'test',
        created_at: 1,
        last_active_at: 1,
        expires_at: now + 60,
      },
    })
    let refreshCalls = 0
    setDevelopmentAuthRefreshAdapter(async (config) => {
      refreshCalls += 1
      return {
        config,
        data: { success: false },
        headers: {},
        status: 401,
        statusText: 'Unauthorized',
      }
    })
    api.defaults.adapter = async (config) => ({
      config,
      data: { success: true, data: { register_enabled: true } },
      headers: {},
      status: 200,
      statusText: 'OK',
    })

    assert.deepEqual(await getStatus(), { register_enabled: true })
    assert.equal(refreshCalls, 0)
  })
})
