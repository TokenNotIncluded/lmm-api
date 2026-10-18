/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { api } from '@/lib/api'

import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from './access'
import {
  nativeProfileCapabilitiesSchema,
  nativeProfileIdSchema,
  nativeProfileSchema,
  nativeProfileUpdateSchema,
  validNativeDisplayName,
  validNativeProfileUpdate,
  type NativeProfile,
  type NativeProfileUpdate,
} from './profile-contract'

export class NativeProfileFailure extends Error {
  constructor(
    readonly kind:
      | 'forbidden'
      | 'not_found'
      | 'conflict'
      | 'invalid'
      | 'unavailable'
      | 'unknown'
      | 'session_changed'
  ) {
    super(`User profile: ${kind}`)
  }
}

function assertProfileScope(scope: NativeUsersScope, signal?: AbortSignal) {
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new NativeProfileFailure('session_changed')
  }
}

function profileFailure(
  error: unknown,
  mutation: boolean
): NativeProfileFailure {
  if (error instanceof NativeProfileFailure) return error
  const response =
    typeof error === 'object' && error !== null && 'response' in error
      ? error.response
      : null
  const status =
    typeof response === 'object' && response !== null && 'status' in response
      ? response.status
      : null
  return new NativeProfileFailure(
    status === 401 || status === 403
      ? 'forbidden'
      : status === 404
        ? 'not_found'
        : status === 409
          ? 'conflict'
          : status === 400
            ? 'invalid'
            : mutation
              ? 'unknown'
              : 'unavailable'
  )
}

function profilePath(userId: string): string {
  if (!nativeProfileIdSchema.safeParse(userId).success) {
    throw new NativeProfileFailure('invalid')
  }
  return `/core/v1/admin/user-profiles/${userId}`
}

export async function readNativeProfileCapabilities(
  scope: NativeUsersScope,
  signal?: AbortSignal
): Promise<void> {
  assertProfileScope(scope, signal)
  try {
    const response = await api.get<unknown>(
      '/core/v1/admin/user-profiles/capabilities',
      nativeUsersOptions(scope, signal)
    )
    assertProfileScope(scope, signal)
    if (!nativeProfileCapabilitiesSchema.safeParse(response.data).success) {
      throw new NativeProfileFailure('unavailable')
    }
  } catch (error) {
    assertProfileScope(scope, signal)
    throw profileFailure(error, false)
  }
}

export async function readNativeProfile(
  scope: NativeUsersScope,
  userId: string,
  signal?: AbortSignal
): Promise<NativeProfile> {
  assertProfileScope(scope, signal)
  const path = profilePath(userId)
  try {
    const response = await api.get<unknown>(
      path,
      nativeUsersOptions(scope, signal)
    )
    assertProfileScope(scope, signal)
    const result = nativeProfileSchema.safeParse(response.data)
    if (!result.success || result.data.user_id !== userId) {
      throw new NativeProfileFailure('unavailable')
    }
    return result.data
  } catch (error) {
    assertProfileScope(scope, signal)
    throw profileFailure(error, false)
  }
}

export async function updateNativeDisplayName(
  scope: NativeUsersScope,
  before: NativeProfile,
  displayName: string
): Promise<NativeProfileUpdate> {
  assertProfileScope(scope)
  if (
    !nativeProfileSchema.safeParse(before).success ||
    !validNativeDisplayName(displayName)
  ) {
    throw new NativeProfileFailure('invalid')
  }
  try {
    const response = await api.patch<unknown>(
      profilePath(before.user_id),
      {
        expected_auth_version: before.auth_version,
        expected_display_name: before.display_name,
        display_name: displayName,
      },
      nativeUsersOptions(scope)
    )
    assertProfileScope(scope)
    const result = nativeProfileUpdateSchema.safeParse(response.data)
    if (
      !result.success ||
      !validNativeProfileUpdate(result.data, before, displayName)
    ) {
      throw new NativeProfileFailure('unknown')
    }
    return result.data
  } catch (error) {
    assertProfileScope(scope)
    throw profileFailure(error, true)
  }
}
