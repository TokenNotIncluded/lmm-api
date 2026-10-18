/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  nativeProfileCapabilitiesSchema,
  nativeProfileIdSchema,
  nativeProfileSchema,
  PROFILE_PROTOCOL,
  validNativeDisplayName,
  validNativeProfileUpdate,
  type NativeProfile,
  type NativeProfileUpdate,
} from './profile-contract'

const profile: NativeProfile = {
  version: 1,
  contract: PROFILE_PROTOCOL,
  user_id: '9007199254740993',
  auth_version: '9007199254740995',
  display_name: 'old',
  platform_role: 'user',
  active: false,
  account_active: false,
}

test('new display names use Unicode scalar counts, allow empty and preserve whitespace', () => {
  for (const value of ['', '  name  ', '😀'.repeat(20), '名'.repeat(20)]) {
    assert.equal(validNativeDisplayName(value), true)
  }
  for (const value of [
    '😀'.repeat(21),
    '\u0000',
    '\u001f',
    '\u007f',
    '\u0080',
    '\u009f',
    '\ud800',
    '\udfff',
  ]) {
    assert.equal(validNativeDisplayName(value), false)
  }
})

test('historical profile names can be repaired without normalizing their CAS value', () => {
  const old = { ...profile, display_name: `\u0000${'😀'.repeat(255)}` }
  assert.equal(nativeProfileSchema.safeParse(old).success, true)
  assert.equal(validNativeDisplayName(old.display_name), false)
  assert.equal(
    nativeProfileSchema.safeParse({ ...old, display_name: 'x'.repeat(257) })
      .success,
    false
  )
})

test('profile IDs preserve i64 precision and reject noncanonical or overflowing numbers', () => {
  assert.equal(
    nativeProfileIdSchema.parse('9223372036854775807'),
    '9223372036854775807'
  )
  for (const value of ['0', '01', '-1', '9223372036854775808', '1e3', 7]) {
    assert.equal(nativeProfileIdSchema.safeParse(value).success, false)
  }
})

test('only the independent exact ordinary-user capability and profile shape can enable editing', () => {
  const capabilities = {
    version: 1,
    contract: PROFILE_PROTOCOL,
    operations: { read: true, update_display_name: true },
    target_platform_roles: ['user'],
    display_name_limits: {
      max_code_points: 20,
      max_utf8_bytes: 80,
      controls_allowed: false,
    },
    preconditions: ['auth_version', 'display_name'],
  }
  assert.equal(
    nativeProfileCapabilitiesSchema.safeParse(capabilities).success,
    true
  )
  for (const value of [
    {},
    { ...capabilities, operations: { read: true, update_display_name: false } },
    { ...capabilities, target_platform_roles: ['admin'] },
    { ...capabilities, extra: true },
  ]) {
    assert.equal(
      nativeProfileCapabilitiesSchema.safeParse(value).success,
      false
    )
  }
  for (const value of [
    { ...profile, platform_role: 'admin' },
    { ...profile, user_id: 7 },
    { ...profile, auth_version: '0' },
    { ...profile, quota: 0 },
  ]) {
    assert.equal(nativeProfileSchema.safeParse(value).success, false)
  }
})

test('only a precise conditional mutation acknowledgement confirms a change or no-op', () => {
  const result: NativeProfileUpdate = {
    version: 1,
    contract: PROFILE_PROTOCOL,
    changed: true,
    profile: { ...profile, display_name: 'new' },
  }
  assert.equal(validNativeProfileUpdate(result, profile, 'new'), true)
  assert.equal(
    validNativeProfileUpdate(
      { ...result, profile: { ...result.profile, auth_version: '2' } },
      profile,
      'new'
    ),
    false
  )
  assert.equal(
    validNativeProfileUpdate({ ...result, changed: false }, profile, 'new'),
    false
  )
  assert.equal(
    validNativeProfileUpdate(
      { ...result, changed: false, profile },
      profile,
      'old'
    ),
    true
  )
})
