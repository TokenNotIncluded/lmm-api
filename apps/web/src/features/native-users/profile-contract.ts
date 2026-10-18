/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

const encoder = new TextEncoder()
const validUnicode = (value: string) => !/[\ud800-\udfff]/u.test(value)
export const nativeProfileIdSchema = z
  .string()
  .refine(
    (value) =>
      value.length <= 19 &&
      /^[1-9][0-9]*$/.test(value) &&
      BigInt(value) <= 9223372036854775807n
  )
const existingName = z
  .string()
  .refine(
    (value) =>
      validUnicode(value) &&
      [...value].length <= 256 &&
      encoder.encode(value).length <= 1024
  )
export const PROFILE_PROTOCOL = 'core.admin-user-profile.v1'

export function hasNativeDisplayNameControls(value: string): boolean {
  return [...value].some((character) => {
    const point = character.codePointAt(0) ?? 0
    return point <= 0x1f || (point >= 0x7f && point <= 0x9f)
  })
}

export function validNativeDisplayName(value: string): boolean {
  return (
    validUnicode(value) &&
    [...value].length <= 20 &&
    encoder.encode(value).length <= 80 &&
    !hasNativeDisplayNameControls(value)
  )
}

export const nativeProfileCapabilitiesSchema = z
  .object({
    version: z.literal(1),
    contract: z.literal(PROFILE_PROTOCOL),
    operations: z
      .object({ read: z.literal(true), update_display_name: z.literal(true) })
      .strict(),
    target_platform_roles: z.tuple([z.literal('user')]),
    display_name_limits: z
      .object({
        max_code_points: z.literal(20),
        max_utf8_bytes: z.literal(80),
        controls_allowed: z.literal(false),
      })
      .strict(),
    preconditions: z.tuple([
      z.literal('auth_version'),
      z.literal('display_name'),
    ]),
  })
  .strict()

export const nativeProfileSchema = z
  .object({
    version: z.literal(1),
    contract: z.literal(PROFILE_PROTOCOL),
    user_id: nativeProfileIdSchema,
    auth_version: nativeProfileIdSchema,
    display_name: existingName,
    platform_role: z.literal('user'),
    active: z.boolean(),
    account_active: z.boolean(),
  })
  .strict()

export const nativeProfileUpdateSchema = z
  .object({
    version: z.literal(1),
    contract: z.literal(PROFILE_PROTOCOL),
    changed: z.boolean(),
    profile: nativeProfileSchema,
  })
  .strict()

export type NativeProfile = z.infer<typeof nativeProfileSchema>
export type NativeProfileUpdate = z.infer<typeof nativeProfileUpdateSchema>

export function validNativeProfileUpdate(
  result: NativeProfileUpdate,
  before: NativeProfile,
  displayName: string
): boolean {
  return (
    result.profile.user_id === before.user_id &&
    result.profile.auth_version === before.auth_version &&
    result.profile.display_name === displayName &&
    result.changed === (before.display_name !== displayName)
  )
}
