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
import { z } from 'zod'

import type { AuthBundle } from '@/stores/auth-store'

const telegramInteger = z.union([
  z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
  z
    .string()
    .regex(/^[1-9][0-9]{0,18}$/)
    .pipe(
      z.string().refine((value) => BigInt(value) <= 9_223_372_036_854_775_807n)
    ),
])

const telegramAuthorizationSchema = z.object({
  id: telegramInteger,
  auth_date: telegramInteger,
  hash: z.string().regex(/^[a-fA-F0-9]{64}$/),
  first_name: z.string().optional(),
  last_name: z.string().optional(),
  username: z.string().optional(),
  photo_url: z.string().optional(),
  lang: z.string().optional(),
})

export type TelegramAuthorization = z.infer<typeof telegramAuthorizationSchema>

export const telegramFlowSchema = z.object({
  flow_token: z.string().regex(/^lmmt_[a-fA-F0-9]{64}$/),
  expires_at: z.number().int().positive(),
})

export type TelegramLoginFlow = z.infer<typeof telegramFlowSchema>

const telegramMFAChallengeSchema = z.object({
  require_2fa: z.literal(true),
  flow_token: z.string().regex(/^lmmf_[a-fA-F0-9]{64}$/),
  expires_at: z.number().int().positive(),
})

export type TelegramMFAChallenge = z.infer<typeof telegramMFAChallengeSchema>

const telegramMFARequiredSchema = z.object({
  require_2fa: z.boolean().optional(),
})

export function parseTelegramMFARequirement(value: unknown): boolean {
  return telegramMFARequiredSchema.parse(value).require_2fa === true
}

export function parseTelegramMFAChallenge(
  value: unknown
): TelegramMFAChallenge | null {
  const result = telegramMFAChallengeSchema.safeParse(value)
  return result.success ? result.data : null
}

export const telegramResponseSchema = z.discriminatedUnion('success', [
  z.object({ success: z.literal(true), data: z.unknown() }),
  z.object({
    success: z.literal(false),
    code: z.string().optional(),
    error: z.object({ code: z.string().optional() }).optional(),
  }),
])

export type TelegramLoginResponse =
  | Extract<z.infer<typeof telegramResponseSchema>, { success: false }>
  | { success: true; data: AuthBundle | TelegramMFAChallenge }

export function pickTelegramAuthorization(
  value: unknown
): TelegramAuthorization | null {
  const result = telegramAuthorizationSchema.safeParse(value)
  return result.success ? result.data : null
}
