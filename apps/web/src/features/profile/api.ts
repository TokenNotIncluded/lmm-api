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

import { parseUsageBuckets } from '@/features/dashboard/lib/usage-bucket'
import { api, type ApiRequestConfig } from '@/lib/api'
import { withAuthenticatedCookieWrite } from '@/lib/auth-session'
import type { CustomOAuthBinding } from '@/lib/oauth'
import { useAuthStore, type LoginSession } from '@/stores/auth-store'
import type { WalletDisplayCurrencyPreference } from '@/stores/wallet-currency-preference-store'

import type { ProfileUsageQueryRange, ProfileUsageRow } from './lib/activity'
import type {
  ApiResponse,
  UserProfile,
  UpdateUserRequest,
  UpdateUserSettingsRequest,
  DeleteAccountRequest,
  CheckinStatusResponse,
  CheckinResponse,
  GiftItem,
  GiftClaimResponse,
  SettlementCurrencyPreference,
} from './types'

export type OAuthGrantSummary = {
  id: string
  client_id: 'lmm' | 'lmm-pi'
  purpose: 'read_only' | 'normal_model' | 'honeypot_only' | string
  scopes: string[]
  budget_credits: number | null
  created_at: number
  expires_at: number
  revoked_at: number | null
  active: boolean
}

export async function getOAuthGrantSummaries(): Promise<
  ApiResponse<OAuthGrantSummary[]>
> {
  const res = await api.get<ApiResponse<OAuthGrantSummary[]>>(
    '/api/user/auth/oauth2/grants',
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return res.data
}

export async function revokeOAuthGrant(id: string): Promise<ApiResponse> {
  const res = await api.delete<ApiResponse>(
    `/api/user/auth/oauth2/grants/${encodeURIComponent(id)}`,
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return res.data
}

export async function revokeAllOAuthGrants(): Promise<
  ApiResponse<{ revoked: number }>
> {
  const res = await api.post<ApiResponse<{ revoked: number }>>(
    '/api/user/auth/oauth2/grants/revoke-all',
    undefined,
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return res.data
}

// ============================================================================
// User Profile APIs
// ============================================================================

/**
 * Get current user profile
 */
export async function getUserProfile(
  config: Pick<
    ApiRequestConfig,
    'timeout' | 'skipBusinessError' | 'skipErrorHandler'
  > = {}
): Promise<ApiResponse<UserProfile>> {
  const { user, session } = useAuthStore.getState().auth
  const res = await api.get<ApiResponse<UserProfile>>('/api/user/self', {
    ...config,
    authScope: { userId: user?.id, sessionId: session?.sid },
  })
  return res.data
}

/**
 * Fetch one bounded window of the current user's token activity.
 */
export async function getProfileUsageWindow(
  params: ProfileUsageQueryRange
): Promise<ProfileUsageRow[]> {
  const res = await api.get<ApiResponse<ProfileUsageRow[]>>('/api/data/self', {
    params: { ...params, default_time: 'day' },
    skipBusinessError: true,
    skipErrorHandler: true,
  })

  if (!res.data.success) {
    throw new Error(res.data.message || 'Unable to load profile activity')
  }
  return parseUsageBuckets(res.data.data)
}

export interface ProfileShareState {
  enabled: boolean
  model_usage_enabled?: boolean
  token?: string
  url?: string
  aggregate_usage_enabled?: boolean
  linked_profiles?: import('./types').LinkedUsageProfile[]
  aggregate_sources?: import('./types').ProfileAggregateSource[]
}

/** Save linked accounts without changing the separate model-sharing permission. */
export async function updateProfileAggregate(
  settings: Partial<import('./types').ProfileAggregateSettings>
): Promise<ProfileShareState> {
  const res = await api.post<ApiResponse<ProfileShareState>>(
    '/api/user/self/profile-share',
    settings,
    { skipBusinessError: true, skipErrorHandler: true }
  )
  if (!res.data.success || !res.data.data) {
    throw new Error('Unable to save linked profiles')
  }
  return res.data.data
}

export async function getProfileShareState(
  signal?: AbortSignal
): Promise<ProfileShareState> {
  const res = await api.get<ApiResponse<ProfileShareState>>(
    '/api/user/self/profile-share',
    { signal, skipBusinessError: true, skipErrorHandler: true }
  )
  if (!res.data.success || !res.data.data) {
    throw new Error(res.data.message || 'Unable to load profile sharing')
  }
  return res.data.data
}

export async function enableProfileShare(
  modelUsageEnabled?: boolean
): Promise<ProfileShareState> {
  const res = await api.post<ApiResponse<ProfileShareState>>(
    '/api/user/self/profile-share',
    modelUsageEnabled === undefined
      ? undefined
      : { model_usage_enabled: modelUsageEnabled }
  )
  if (!res.data.success || !res.data.data) {
    throw new Error(res.data.message || 'Unable to enable profile sharing')
  }
  return res.data.data
}

export async function disableProfileShare(): Promise<ProfileShareState> {
  const res = await api.delete<ApiResponse<ProfileShareState>>(
    '/api/user/self/profile-share'
  )
  if (!res.data.success || !res.data.data) {
    throw new Error(res.data.message || 'Unable to disable profile sharing')
  }
  return res.data.data
}

/**
 * Update user profile
 */
export async function updateUserProfile(
  data: UpdateUserRequest
): Promise<ApiResponse> {
  const res = data.password
    ? await withAuthenticatedCookieWrite((config) =>
        api.put('/api/user/self', data, {
          ...config,
          acceptAuthRotation: true,
        })
      )
    : await api.put('/api/user/self', data)
  return res.data
}

/**
 * Update user settings
 */
export async function updateUserSettings(
  data: UpdateUserSettingsRequest
): Promise<ApiResponse> {
  const res = await api.put('/api/user/setting', data)
  return res.data
}

/**
 * Update interface language preference
 */
export async function updateUserLanguage(
  language: string
): Promise<ApiResponse> {
  const res = await api.put('/api/user/self', { language })
  return res.data
}

/** Save only the customer fiat preference, without changing language or quota. */
export async function updateSettlementCurrency(
  settlementCurrency: SettlementCurrencyPreference,
  signal?: AbortSignal
): Promise<ApiResponse> {
  const res = await api.put(
    '/api/user/self',
    { settlement_currency: settlementCurrency },
    { signal, skipBusinessError: true, skipErrorHandler: true }
  )
  return res.data
}

/** Save only the display preference; settlement and stored quota are unchanged. */
export async function updateWalletDisplayCurrency(
  walletDisplayCurrency: WalletDisplayCurrencyPreference,
  signal?: AbortSignal
): Promise<ApiResponse> {
  const res = await api.put(
    '/api/user/self',
    { wallet_display_currency: walletDisplayCurrency },
    { signal, skipBusinessError: true, skipErrorHandler: true }
  )
  return res.data
}

/**
 * Delete user account
 */
export async function deleteUserAccount(
  data?: DeleteAccountRequest
): Promise<ApiResponse> {
  const res = await api.delete('/api/user/self', { data })
  return res.data
}

// ============================================================================
// Account Binding APIs
// ============================================================================

/**
 * Send email verification code
 */
export async function sendEmailVerification(
  email: string,
  turnstileToken?: string
): Promise<ApiResponse> {
  const params = new URLSearchParams({ email })
  if (turnstileToken) {
    params.append('turnstile', turnstileToken)
  }
  const res = await api.get(`/api/verification?${params}`)
  return res.data
}

/**
 * Bind email account
 */
export async function bindEmail(
  email: string,
  code: string
): Promise<ApiResponse> {
  const res = await api.post('/api/oauth/email/bind', {
    email,
    code,
  })
  return res.data
}

/**
 * Bind WeChat account
 */
export async function bindWeChat(code: string): Promise<ApiResponse> {
  const res = await api.post(
    '/api/oauth/wechat/bind',
    { code },
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return res.data
}

const telegramBindFlowSchema = z
  .object({
    flow_token: z.string().regex(/^lmmt_[0-9a-fA-F]{64}$/),
    callback_url: z.string().max(4096),
    expires_at: z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
  })
  .superRefine((flow, context) => {
    const now = Math.floor(Date.now() / 1000)
    if (flow.expires_at <= now) {
      context.addIssue({
        code: 'custom',
        path: ['expires_at'],
        message: 'Invalid Telegram binding lifetime',
      })
    }
    const expected = `${window.location.origin}/api/oauth/telegram/bind/${flow.flow_token}`
    if (flow.callback_url !== expected) {
      context.addIssue({
        code: 'custom',
        path: ['callback_url'],
        message: 'Invalid Telegram binding callback',
      })
    }
  })

const telegramBindStartSchema = z.discriminatedUnion('success', [
  z.object({ success: z.literal(true), data: telegramBindFlowSchema }),
  z.object({
    success: z.literal(false),
    code: z.string().max(128).optional(),
    error: z.object({ code: z.string().max(128) }).optional(),
  }),
])

export type TelegramBindFlow = z.infer<typeof telegramBindFlowSchema>

export async function startTelegramBind(): Promise<
  z.infer<typeof telegramBindStartSchema>
> {
  const res = await withAuthenticatedCookieWrite((config) =>
    api.post<unknown>('/api/oauth/telegram/bind/start', undefined, {
      ...config,
      headers: config.authScope.sessionId
        ? { 'X-Auth-Session': config.authScope.sessionId }
        : undefined,
      skipBusinessError: true,
      skipErrorHandler: true,
    })
  )
  return telegramBindStartSchema.parse(res.data)
}

// ============================================================================
// Login Session APIs
// ============================================================================

interface LoginSessionsResponse extends ApiResponse<LoginSession[]> {
  session_auto_logout?: boolean
}

export async function getLoginSessions(): Promise<LoginSessionsResponse> {
  const res = await api.get('/api/user/sessions')
  return res.data
}

export async function updateLoginSessionSettings(
  sessionAutoLogout: boolean
): Promise<ApiResponse> {
  const res = await api.put(
    '/api/user/sessions/settings',
    { session_auto_logout: sessionAutoLogout },
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return res.data
}

export async function revokeLoginSession(sid: string): Promise<ApiResponse> {
  const res = await api.delete(`/api/user/sessions/${encodeURIComponent(sid)}`)
  return res.data
}

export async function revokeOtherLoginSessions(): Promise<ApiResponse> {
  const res = await api.post('/api/user/sessions/revoke-others')
  return res.data
}

// ============================================================================
// Custom OAuth Binding APIs
// ============================================================================

export type { CustomOAuthBinding } from '@/lib/oauth'

/**
 * Get current user's custom OAuth bindings
 */
export async function getSelfOAuthBindings(): Promise<
  ApiResponse<CustomOAuthBinding[]>
> {
  const res = await api.get('/api/user/oauth/bindings')
  return res.data
}

/**
 * Unbind a custom OAuth provider for current user
 */
export async function unbindCustomOAuth(
  providerId: number
): Promise<ApiResponse> {
  const res = await api.delete(`/api/user/oauth/bindings/${providerId}`)
  return res.data
}

/**
 * Unbind a built-in OAuth provider for the current user
 */
export async function unbindBuiltInOAuth(
  bindingType: string
): Promise<ApiResponse> {
  const res = await api.delete(
    `/api/user/bindings/${encodeURIComponent(bindingType)}`
  )
  return res.data
}

// ============================================================================
// Checkin APIs
// ============================================================================

/**
 * Get checkin status for a specific month
 */
export async function getCheckinStatus(
  month: string
): Promise<ApiResponse<CheckinStatusResponse>> {
  const res = await api.get(`/api/user/checkin?month=${month}`)
  return res.data
}

/**
 * Perform daily checkin
 */
export async function performCheckin(
  turnstileToken?: string
): Promise<ApiResponse<CheckinResponse>> {
  const url = turnstileToken
    ? `/api/user/checkin?turnstile=${encodeURIComponent(turnstileToken)}`
    : '/api/user/checkin'
  const res = await api.post(url, undefined, {
    // The check-in card presents verification failures itself so that the
    // challenge flow does not produce a duplicate global toast.
    skipBusinessError: true,
  })
  return res.data
}

// ============================================================================
// Compensation Gift APIs
// ============================================================================

/**
 * List gifts currently visible to the user with claim/eligibility status
 */
export async function getAvailableGifts(): Promise<ApiResponse<GiftItem[]>> {
  const res = await api.get('/api/user/gift')
  return res.data
}

/**
 * Claim a compensation gift. Idempotent: repeated claims return 200 with
 * `already_claimed=true` and quota is credited exactly once.
 */
export async function claimGift(
  giftId: number
): Promise<ApiResponse<GiftClaimResponse>> {
  const res = await api.post(`/api/user/gift/${giftId}/claim`)
  return res.data
}
