/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  loadNotificationPreferences,
  notificationPatch,
  notificationScope,
  saveNotificationPreferences,
  type NotificationPreferences,
} from './notification-preferences'

const originalGet = api.get
const originalPut = api.put
afterEach(() => {
  api.get = originalGet
  api.put = originalPut
  useAuthStore.getState().auth.reset('idle')
})
function owner(id = 7) {
  useAuthStore
    .getState()
    .auth.setUser({ id, username: 'notifications-test', role: 1 })
  const scope = notificationScope()
  assert.ok(scope)
  return scope
}
function view(admin = false): NotificationPreferences {
  return {
    user_id: 7,
    notify_type: 'email',
    quota_warning_enabled: true,
    quota_warning_threshold: 0,
    channel_notify_enabled: false,
    upstream_model_update_notify_enabled: false,
    notification_email: 'verified@example.invalid',
    verified_email_available: true,
    admin_notifications_allowed: admin,
    site: {
      revision: 1,
      quota_warning_enabled: true,
      quota_warning_threshold: 500000,
      channel_notify_enabled: true,
      upstream_model_update_notify_enabled: true,
    },
  }
}
test('native patch sends only notification keys and never role or money authority', () => {
  const current = view()
  const patch = notificationPatch(current, {
    ...current,
    quota_warning_enabled: false,
    quota_warning_threshold: 123,
    allow_key_bypass_ip_policy: true,
    price: 0,
    user_id: 8,
  } as typeof current)
  assert.deepEqual(patch, {
    notify_type: 'email',
    notification_email: '',
    quota_warning_enabled: false,
    quota_warning_threshold: 123,
  })
  assert.deepEqual(
    notificationPatch(view(true), { ...current, channel_notify_enabled: true }),
    {
      notify_type: 'email',
      notification_email: '',
      quota_warning_enabled: true,
      quota_warning_threshold: 0,
      channel_notify_enabled: true,
      upstream_model_update_notify_enabled: false,
    }
  )
})
test('threshold rejects fractions, negative values, infinity and unsafe integers', () => {
  for (const threshold of [
    -1,
    0.5,
    NaN,
    Infinity,
    Number.MAX_SAFE_INTEGER + 1,
  ]) {
    assert.throws(() =>
      notificationPatch(view(), {
        ...view(),
        quota_warning_threshold: threshold,
      })
    )
  }
  assert.equal(notificationPatch(view(), view()).quota_warning_threshold, 0)
})
test('read binds response to the current account and session', async () => {
  const scope = owner()
  api.get = (async (path, config) => {
    assert.equal(path, '/api/user/notifications')
    assert.deepEqual(config?.authScope, scope)
    assert.equal(config?.skipAuthRefresh, true)
    return { data: { success: true, data: view() } }
  }) as typeof api.get
  assert.equal(
    (await loadNotificationPreferences(scope)).notification_email,
    'verified@example.invalid'
  )
})
test('a late response from the previous account is discarded', async () => {
  const scope = owner()
  api.get = (async () => {
    owner(8)
    return { data: { success: true, data: view() } }
  }) as typeof api.get
  await assert.rejects(loadNotificationPreferences(scope), /session changed/)
})
test('one successful native save verifies its owner and preserves explicit disabled preferences', async () => {
  const scope = owner()
  let calls = 0
  api.put = (async (path, body, config) => {
    calls++
    assert.equal(path, '/api/user/notifications')
    assert.deepEqual(config?.authScope, scope)
    assert.equal(config?.skipAuthRefresh, true)
    assert.equal(body.quota_warning_enabled, false)
    return {
      data: {
        success: true,
        data: { ...view(), quota_warning_enabled: false },
      },
    }
  }) as typeof api.put
  const saved = await saveNotificationPreferences(scope, view(), {
    ...view(),
    quota_warning_enabled: false,
  })
  assert.equal(saved.quota_warning_enabled, false)
  assert.equal(calls, 1)
})
test('unknown write outcome is not retried and a switched account cannot submit', async () => {
  const scope = owner()
  let calls = 0
  api.put = (async () => {
    calls++
    throw new Error('connection lost')
  }) as typeof api.put
  await assert.rejects(saveNotificationPreferences(scope, view(), view()))
  assert.equal(calls, 1)
  owner(8)
  await assert.rejects(saveNotificationPreferences(scope, view(), view()))
  assert.equal(calls, 1)
})
test('wrong-owner and malformed successful responses are not accepted as saved', async () => {
  const scope = owner()
  for (const data of [
    { ...view(), user_id: 8 },
    { ...view(), quota_warning_enabled: 'true' },
    { ...view(), site: {} },
  ]) {
    api.put = (async () => ({
      data: { success: true, data },
    })) as typeof api.put
    await assert.rejects(saveNotificationPreferences(scope, view(), view()))
  }
})
