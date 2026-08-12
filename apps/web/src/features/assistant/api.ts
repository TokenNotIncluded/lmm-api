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
import type { QuotaDataItem } from '@/features/dashboard/types'
import type { PricingData } from '@/features/pricing/types'
import type { PlanRecord } from '@/features/subscriptions/types'
import { api } from '@/lib/api'

type AssistantChatPayload = {
  choices?: Array<{
    message?: {
      content?: string
    }
  }>
  error?: {
    message?: string
  }
  message?: string
  lmm_assistant_action?: unknown
}

export type AssistantChatMessage = {
  role: 'user' | 'assistant'
  content: string
}

const ASSISTANT_CONVERSATION_MAX_ITEMS = 12
const ASSISTANT_CONVERSATION_MAX_RUNES = 12_000
const ASSISTANT_MESSAGE_MAX_RUNES = 4_000

export type AssistantFundingStatus = {
  mode: 'super_administrator'
}

export type AssistantStatus = {
  enabled: boolean
  model: string
  funding: AssistantFundingStatus
  developer_access_granted: boolean
  access_level?: string
  trust_level?: number
  role?: number
  is_admin?: boolean
  is_root?: boolean
  capabilities?: {
    public_assistant?: boolean
    account?: boolean
    developer_tools?: boolean
    personal_ip_allowlist?: boolean
    usage_discount?: boolean
    admin_config?: boolean
    admin_pricing?: boolean
  }
}

export type AssistantL1RecommendationAction = {
  type: 'l1_recommendation'
  user_statement: string
  recommendation: string
  confirmation_token: string
}

export type AssistantAccountDisableAction = {
  type: 'account_disable_request'
  target_user_id: number
  target_username: string
  reason: string
  confirmation_token: string
}

export type AssistantAdminConfigPreview = {
  key: string
  label: string
  old_value: string
  new_value: string
}

export type AssistantAdminPricingPreview = {
  model_id: string
  old: Record<string, unknown>
  next: Record<string, unknown>
}

export type AssistantAdminConfigChangeAction = {
  type: 'admin_config_change'
  confirmation_token: string
  requires_confirmation: true
  expires_in_seconds: number
  changes: AssistantAdminConfigPreview[]
}

export type AssistantAdminPricingChangeAction = {
  type: 'admin_pricing_change'
  confirmation_token: string
  requires_confirmation: true
  expires_in_seconds: number
  pricing: AssistantAdminPricingPreview
}

export type AssistantAdminChangeAction =
  | AssistantAdminConfigChangeAction
  | AssistantAdminPricingChangeAction

export type AssistantAction =
  | AssistantL1RecommendationAction
  | AssistantAccountDisableAction
  | AssistantAdminChangeAction

export type AssistantCreatedKey = {
  id: number
  name: string
  group: string
  expired_time: number
  card: AssistantPrivateCard
}

export type AssistantPrivateCard = {
  id: string
  label?: string
}

export type AssistantSecureCardView = {
  id?: string
  type?: string
  label?: string
  owner: 'self' | 'protected'
  shield: boolean
}

export type AssistantConversationHistoryMessage = {
  id: number
  role: 'user' | 'assistant' | 'secure_card'
  content: string
  created_at: number
  cards?: AssistantSecureCardView[]
}

export type AssistantConversationHistorySummary = {
  id: number
  title: string
  last_message_preview: string
  created_at: number
  updated_at: number
  owner: 'self' | 'lower_level_user'
  privacy_notice: string
}

export type AssistantConversationHistoryItem =
  AssistantConversationHistorySummary

export type AssistantConversationHistory = {
  conversations: AssistantConversationHistorySummary[]
  privacy_notice?: string
}

export type AssistantConversationHistoryDetail = {
  conversation: AssistantConversationHistorySummary
  messages: AssistantConversationHistoryMessage[]
  privacy_notice: string
}

export type AssistantHandoff = {
  id: number
  user_id: number
  source: 'handoff'
  intent: 'human_support'
  message: string
  status: 'pending' | 'resolved'
  admin_user_id: number
  admin_note: string
  created_at: number
  resolved_at: number
  username?: string
  email?: string
}

export type AssistantIntent =
  | 'onboarding'
  | 'plan_purchase'
  | 'api_key'
  | 'client_setup'
  | 'cost'
  | 'bounty'
  | 'usage'
  | 'models'
  | 'invitation'
  | 'human_support'
  | 'other'

export type AssistantIntentSummary = {
  intent: AssistantIntent
  count: number
}

export type AssistantProfileSummary = {
  profile: string
  count: number
}

export type AssistantFundingSummary = {
  start_timestamp: number
  end_timestamp: number
  requests: number
  prompt_tokens: number
  completion_tokens: number
  total_tokens: number
  quota: number
  cost_usd: number
  remaining_quota: number
  remaining_usd: number
}

export type AssistantReply = {
  content: string
  intent?: AssistantIntent
  action?: AssistantAction
}

export type AssistantPlanOffers = {
  ok: boolean
  developer_access_granted: boolean
  read_only: boolean
  checkout_available: boolean
  payment_hidden: boolean
  payment_compliance_confirmed: boolean
  plans: PlanRecord[]
  topup_discounts: Record<string, number>
  message?: string
  next_step?: string
  error?: string
}

const ASSISTANT_INTENTS = new Set<AssistantIntent>([
  'onboarding',
  'plan_purchase',
  'api_key',
  'client_setup',
  'cost',
  'bounty',
  'usage',
  'models',
  'invitation',
  'human_support',
  'other',
])

type AssistantAPIResponse<T> = {
  success: boolean
  data?: T
  message?: string
}

function requireAssistantData<T>(
  payload: AssistantAPIResponse<T>,
  fallback: string
): T {
  if (!payload.success || payload.data === undefined) {
    throw new Error(payload.message || fallback)
  }
  return payload.data
}

export function parseAssistantReply(payload: AssistantChatPayload): string {
  const content = payload.choices?.[0]?.message?.content?.trim()
  if (content) return content
  throw new Error(
    payload.error?.message || payload.message || 'Assistant returned no answer'
  )
}

export function parseAssistantIntent(
  value: unknown
): AssistantIntent | undefined {
  if (typeof value !== 'string') return undefined
  const intent = value.trim().toLowerCase() as AssistantIntent
  return ASSISTANT_INTENTS.has(intent) ? intent : undefined
}

export function parseAssistantAction(
  value: unknown
): AssistantAction | undefined {
  if (!value || typeof value !== 'object') return undefined
  const action = value as Record<string, unknown>
  const confirmationToken =
    typeof action.confirmation_token === 'string'
      ? action.confirmation_token.trim()
      : ''
  if (!confirmationToken) return undefined

  if (
    action.type === 'admin_config_change' &&
    action.requires_confirmation === true &&
    typeof action.expires_in_seconds === 'number' &&
    Number.isInteger(action.expires_in_seconds) &&
    action.expires_in_seconds > 0 &&
    Array.isArray(action.changes)
  ) {
    const changes = action.changes
      .map((item) => {
        if (!item || typeof item !== 'object') return null
        const change = item as Record<string, unknown>
        if (
          typeof change.key !== 'string' ||
          typeof change.label !== 'string' ||
          typeof change.old_value !== 'string' ||
          typeof change.new_value !== 'string'
        ) {
          return null
        }
        const key = change.key.trim()
        const label = change.label.trim()
        if (!key || !label) return null
        return {
          key,
          label,
          old_value: change.old_value,
          new_value: change.new_value,
        }
      })
      .filter(
        (change): change is AssistantAdminConfigPreview => change !== null
      )
    if (changes.length === action.changes.length && changes.length > 0) {
      return {
        type: 'admin_config_change',
        confirmation_token: confirmationToken,
        requires_confirmation: true,
        expires_in_seconds: action.expires_in_seconds,
        changes,
      }
    }
  }

  if (
    action.type === 'admin_pricing_change' &&
    action.requires_confirmation === true &&
    typeof action.expires_in_seconds === 'number' &&
    Number.isInteger(action.expires_in_seconds) &&
    action.expires_in_seconds > 0 &&
    action.pricing &&
    typeof action.pricing === 'object'
  ) {
    const pricing = action.pricing as Record<string, unknown>
    if (
      typeof pricing.model_id === 'string' &&
      pricing.old &&
      typeof pricing.old === 'object' &&
      pricing.next &&
      typeof pricing.next === 'object'
    ) {
      const modelId = pricing.model_id.trim()
      if (modelId) {
        return {
          type: 'admin_pricing_change',
          confirmation_token: confirmationToken,
          requires_confirmation: true,
          expires_in_seconds: action.expires_in_seconds,
          pricing: {
            model_id: modelId,
            old: pricing.old as Record<string, unknown>,
            next: pricing.next as Record<string, unknown>,
          },
        }
      }
    }
  }

  if (
    action.type === 'l1_recommendation' &&
    typeof action.user_statement === 'string' &&
    typeof action.recommendation === 'string'
  ) {
    const userStatement = action.user_statement.trim()
    const recommendation = action.recommendation.trim()
    if (!userStatement || !recommendation) return undefined
    return {
      type: 'l1_recommendation',
      user_statement: userStatement,
      recommendation,
      confirmation_token: confirmationToken,
    }
  }

  if (
    action.type === 'account_disable_request' &&
    typeof action.target_user_id === 'number' &&
    Number.isInteger(action.target_user_id) &&
    action.target_user_id > 0 &&
    typeof action.target_username === 'string' &&
    typeof action.reason === 'string'
  ) {
    const targetUsername = action.target_username.trim()
    const reason = action.reason.trim()
    if (!targetUsername || !reason) return undefined
    return {
      type: 'account_disable_request',
      target_user_id: action.target_user_id,
      target_username: targetUsername,
      reason,
      confirmation_token: confirmationToken,
    }
  }
  return undefined
}

function normalizedAssistantHistoryMessage(
  message: AssistantChatMessage
): AssistantChatMessage | null {
  const content = message.content.trim()
  if (!content) return null
  return {
    role: message.role,
    content: [...content].slice(0, ASSISTANT_MESSAGE_MAX_RUNES).join(''),
  }
}

export function buildAssistantConversation(
  history: AssistantChatMessage[],
  currentMessage: string
): AssistantChatMessage[] {
  const current = currentMessage.trim()
  const currentRunes = [...current]
  if (!current || currentRunes.length > ASSISTANT_MESSAGE_MAX_RUNES) {
    throw new Error(
      `Assistant message must be between 1 and ${ASSISTANT_MESSAGE_MAX_RUNES} characters`
    )
  }

  const conversation: AssistantChatMessage[] = [
    { role: 'user', content: current },
  ]
  let totalRunes = currentRunes.length
  for (let index = history.length - 1; index >= 0; index -= 1) {
    if (conversation.length >= ASSISTANT_CONVERSATION_MAX_ITEMS) break
    const message = normalizedAssistantHistoryMessage(history[index])
    if (!message) continue
    const messageRunes = [...message.content].length
    if (totalRunes + messageRunes > ASSISTANT_CONVERSATION_MAX_RUNES) break
    conversation.unshift(message)
    totalRunes += messageRunes
  }

  while (conversation[0]?.role === 'assistant') conversation.shift()
  return conversation
}

export async function sendAssistantMessage(
  message: string,
  history: AssistantChatMessage[] = []
): Promise<AssistantReply> {
  const normalizedMessage = message.trim()
  const messages = buildAssistantConversation(history, normalizedMessage)
  const response = await api.post<AssistantChatPayload>(
    '/api/assistant/chat',
    { message: normalizedMessage, messages },
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return {
    content: parseAssistantReply(response.data),
    intent: parseAssistantIntent(response.headers['x-lmm-assistant-intent']),
    action: parseAssistantAction(response.data.lmm_assistant_action),
  }
}

export async function submitAssistantAccountDisableRequest(input: {
  target_user_id: number
  reason: string
  confirmation_token: string
}): Promise<unknown> {
  const response = await api.post<AssistantAPIResponse<unknown>>(
    '/api/user/account-action-requests',
    { ...input, confirmed: true },
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return requireAssistantData(
    response.data,
    'Unable to submit the account safety request'
  )
}

export async function submitAssistantAdminChange(
  confirmationToken: string
): Promise<{ applied: boolean; kind: string }> {
  const response = await api.post<
    AssistantAPIResponse<{ applied: boolean; kind: string }>
  >(
    '/api/assistant/admin/apply',
    { confirmation_token: confirmationToken, confirmed: true },
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return requireAssistantData(
    response.data,
    'Unable to apply the administrator change'
  )
}

export async function getAssistantStatus(): Promise<AssistantStatus> {
  const response = await api.get<AssistantAPIResponse<AssistantStatus>>(
    '/api/assistant/status',
    {
      disableDuplicate: true,
      skipBusinessError: true,
      skipErrorHandler: true,
    }
  )
  return requireAssistantData(response.data, 'Unable to load assistant status')
}

export async function getAssistantAvailableModels(): Promise<string[]> {
  const response = await api.get<AssistantAPIResponse<string[]>>(
    '/api/user/models',
    {
      skipBusinessError: true,
      skipErrorHandler: true,
    }
  )
  return requireAssistantData(response.data, 'Unable to load available models')
}

export async function getAssistantPricing(): Promise<PricingData> {
  const response = await api.get<PricingData>('/api/assistant/pricing', {
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  if (!response.data.success) {
    throw new Error(response.data.message || 'Unable to load live pricing')
  }
  return response.data
}

export async function getAssistantPlanOffers(): Promise<AssistantPlanOffers> {
  const response = await api.get<AssistantAPIResponse<AssistantPlanOffers>>(
    '/api/assistant/offers',
    {
      skipBusinessError: true,
      skipErrorHandler: true,
    }
  )
  const offers = requireAssistantData(
    response.data,
    'Unable to load live plan offers'
  )
  if (!offers.ok) {
    throw new Error(offers.error || 'Unable to load live plan offers')
  }
  return offers
}

export async function getAssistantUsageData(
  days: 7 | 30 | 90
): Promise<QuotaDataItem[]> {
  const endTimestamp = Math.floor(Date.now() / 1000)
  const response = await api.get<AssistantAPIResponse<QuotaDataItem[]>>(
    '/api/data/self',
    {
      params: {
        start_timestamp: endTimestamp - days * 24 * 60 * 60,
        end_timestamp: endTimestamp,
        default_time: 'day',
      },
      skipBusinessError: true,
      skipErrorHandler: true,
    }
  )
  return requireAssistantData(response.data, 'Failed to fetch usage')
}

export async function createAssistantDefaultKey(
  name: string,
  group: string
): Promise<AssistantCreatedKey> {
  const response = await api.post<AssistantAPIResponse<AssistantCreatedKey>>(
    '/api/assistant/tools/create-key',
    { confirmed: true, name, group },
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return requireAssistantData(response.data, 'Unable to create API key')
}

export async function revealAssistantPrivateCard(id: string): Promise<string> {
  const response = await api.get<
    AssistantAPIResponse<{
      payload?: Record<string, string>
    }>
  >(`/api/assistant/cards/${encodeURIComponent(id)}/reveal`, {
    disableDuplicate: true,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  const data = requireAssistantData(
    response.data,
    'Unable to retrieve the private credential'
  )
  const value = data.payload?.api_key?.trim()
  if (!value) throw new Error('Unable to retrieve the private credential')
  return value
}

export async function getAssistantConversationHistory(): Promise<AssistantConversationHistory> {
  const response = await api.get<
    AssistantAPIResponse<AssistantConversationHistory>
  >('/api/assistant/conversations', {
    disableDuplicate: true,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return requireAssistantData(
    response.data,
    'Unable to load conversation history'
  )
}

export async function getAssistantConversationHistoryDetail(
  id: number
): Promise<AssistantConversationHistoryDetail> {
  const response = await api.get<
    AssistantAPIResponse<AssistantConversationHistoryDetail>
  >(`/api/assistant/conversations/${encodeURIComponent(id)}`, {
    disableDuplicate: true,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return requireAssistantData(
    response.data,
    'Unable to load conversation details'
  )
}

export async function getAssistantHandoff(): Promise<AssistantHandoff | null> {
  const response = await api.get<AssistantAPIResponse<AssistantHandoff | null>>(
    '/api/assistant/handoffs/self',
    {
      disableDuplicate: true,
      skipBusinessError: true,
      skipErrorHandler: true,
    }
  )
  return requireAssistantData(response.data, 'Unable to load support request')
}

export async function submitAssistantHandoff(
  message: string
): Promise<AssistantHandoff> {
  const response = await api.post<AssistantAPIResponse<AssistantHandoff>>(
    '/api/assistant/handoffs',
    { confirmed: true, message },
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return requireAssistantData(response.data, 'Unable to contact support')
}

export async function listAssistantHandoffs(
  status: 'pending' | 'resolved' = 'pending'
): Promise<AssistantHandoff[]> {
  const response = await api.get<AssistantAPIResponse<AssistantHandoff[]>>(
    '/api/assistant/admin/handoffs',
    {
      params: { status },
      skipBusinessError: true,
      skipErrorHandler: true,
    }
  )
  return requireAssistantData(response.data, 'Unable to load support requests')
}

export async function getAssistantIntentSummary(
  days = 30
): Promise<AssistantIntentSummary[]> {
  const response = await api.get<
    AssistantAPIResponse<AssistantIntentSummary[]>
  >('/api/assistant/admin/intents', {
    params: { days },
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return requireAssistantData(response.data, 'Unable to load intent summary')
}

export async function getAssistantProfileSummary(
  days = 30
): Promise<AssistantProfileSummary[]> {
  const response = await api.get<
    AssistantAPIResponse<AssistantProfileSummary[]>
  >('/api/assistant/admin/profiles', {
    params: { days },
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return requireAssistantData(response.data, 'Unable to load profile summary')
}

export async function getAssistantFundingSummary(
  days = 30
): Promise<AssistantFundingSummary> {
  const response = await api.get<AssistantAPIResponse<AssistantFundingSummary>>(
    '/api/assistant/admin/funding',
    {
      params: { days },
      skipBusinessError: true,
      skipErrorHandler: true,
    }
  )
  return requireAssistantData(response.data, 'Unable to load assistant funding')
}

export async function resolveAssistantHandoff(
  id: number,
  note: string
): Promise<AssistantHandoff> {
  const response = await api.post<AssistantAPIResponse<AssistantHandoff>>(
    `/api/assistant/admin/handoffs/${id}/resolve`,
    { note },
    { skipBusinessError: true, skipErrorHandler: true }
  )
  return requireAssistantData(
    response.data,
    'Unable to resolve support request'
  )
}
