/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { readNativeUsersScope } from './access'
import {
  NativeProfileFailure,
  readNativeProfile,
  readNativeProfileCapabilities,
  updateNativeDisplayName,
} from './profile-api'
import { PROFILE_PROTOCOL, type NativeProfile } from './profile-contract'

const originalGet = api.get
const originalPatch = api.patch
const profile: NativeProfile = {
  version: 1,
  contract: PROFILE_PROTOCOL,
  user_id: '9007199254740993',
  auth_version: '9007199254740995',
  display_name: '  historical\nname  ',
  platform_role: 'user',
  active: false,
  account_active: false,
}

function login(sid = 'synthetic-profile-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-profile-access',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 7, role: 10, username: 'synthetic' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
  const scope = readNativeUsersScope()
  assert.ok(scope)
  return scope
}
afterEach(() => {
  api.get = originalGet
  api.patch = originalPatch
  useAuthStore.getState().auth.reset('idle')
})

test('profile reads stay separate from native read-v1 and preserve exact target/version strings', async () => {
  const scope = login()
  api.get = (async (url: string, options: Record<string, unknown>) => {
    assert.equal(url, `/core/v1/admin/user-profiles/${profile.user_id}`)
    assert.deepEqual(options.authScope, {
      userId: 7,
      sessionId: scope.sessionId,
    })
    assert.equal(options.skipAuthRefresh, true)
    return { data: profile }
  }) as typeof api.get
  assert.deepEqual(await readNativeProfile(scope, profile.user_id), profile)
})

test('capability failures and privileged/mismatched profile DTOs never become editable data', async () => {
  const scope = login()
  api.get = (async () => ({
    data: { version: 1, contract: PROFILE_PROTOCOL, operations: {} },
  })) as typeof api.get
  await assert.rejects(
    readNativeProfileCapabilities(scope),
    (error) =>
      error instanceof NativeProfileFailure && error.kind === 'unavailable'
  )
  for (const value of [
    { ...profile, platform_role: 'admin' },
    { ...profile, user_id: '8' },
  ]) {
    api.get = (async () => ({ data: value })) as typeof api.get
    await assert.rejects(
      readNativeProfile(scope, profile.user_id),
      /unavailable/
    )
  }
})

test('one PATCH contains only the exact old CAS and untrimmed new name', async () => {
  const scope = login()
  let writes = 0
  api.patch = (async (
    url: string,
    body: unknown,
    options: Record<string, unknown>
  ) => {
    writes += 1
    assert.equal(url, `/core/v1/admin/user-profiles/${profile.user_id}`)
    assert.deepEqual(body, {
      expected_auth_version: profile.auth_version,
      expected_display_name: profile.display_name,
      display_name: '  new  ',
    })
    assert.equal(options.skipAuthRefresh, true)
    return {
      data: {
        version: 1,
        contract: PROFILE_PROTOCOL,
        changed: true,
        profile: { ...profile, display_name: '  new  ' },
      },
    }
  }) as typeof api.patch
  assert.equal(
    (await updateNativeDisplayName(scope, profile, '  new  ')).changed,
    true
  )
  assert.equal(writes, 1)
})

for (const failure of [
  400,
  401,
  403,
  404,
  409,
  503,
  'transport',
  'malformed',
] as const) {
  test(`${failure} PATCH is never automatically replayed or inferred successful`, async () => {
    const scope = login()
    let writes = 0
    api.patch = (async () => {
      writes += 1
      if (failure === 'malformed') {
        return { data: { ...profile, display_name: 'new' } }
      }
      if (typeof failure === 'number') {
        throw Object.assign(new Error(`HTTP ${failure}`), {
          response: { status: failure },
        })
      }
      throw new Error('lost response')
    }) as typeof api.patch
    const expected =
      failure === 400
        ? 'invalid'
        : failure === 401 || failure === 403
          ? 'forbidden'
          : failure === 404
            ? 'not_found'
            : failure === 409
              ? 'conflict'
              : 'unknown'
    await assert.rejects(
      updateNativeDisplayName(scope, profile, 'new'),
      (error) =>
        error instanceof NativeProfileFailure && error.kind === expected
    )
    assert.equal(writes, 1)
  })
}

test('SID changes and invalid new names reject before dispatch', async () => {
  const scope = login()
  let writes = 0
  api.patch = (async () => {
    writes += 1
    throw new Error('unexpected')
  }) as typeof api.patch
  await assert.rejects(
    updateNativeDisplayName(scope, profile, 'x'.repeat(21)),
    /invalid/
  )
  login('replacement-session')
  await assert.rejects(
    updateNativeDisplayName(scope, profile, 'new'),
    /session_changed/
  )
  assert.equal(writes, 0)
})
