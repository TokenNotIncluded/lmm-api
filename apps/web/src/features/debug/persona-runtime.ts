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
import {
  AxiosError,
  type AxiosAdapter,
  type AxiosResponse,
  type InternalAxiosRequestConfig,
} from 'axios'

import {
  applyAuthBundle,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { api } from '@/lib/http-client'
import { ROLE } from '@/lib/roles'
import type { AuthBundle, AuthUser, TrustLevelInfo } from '@/stores/auth-store'

export const DEBUG_PERSONA_IDS = ['l0', 'l1', 'admin'] as const
export type DebugPersonaId = (typeof DEBUG_PERSONA_IDS)[number]

type MockConversation = {
  id: number
  userId: number
  title: string
  preview: string
  createdAt: number
  messages: Array<{
    id: number
    role: 'user' | 'assistant'
    content: string
    created_at: number
  }>
}

type DebugState = {
  activePersona: DebugPersonaId
  conversations: MockConversation[]
}

type DebugEvent = {
  persona: DebugPersonaId
}

const DEBUG_EVENT = 'lmm:persona-debug-change'
const BLOCKED_DEBUG_REQUEST = 'PERSONA_DEBUG_UNMOCKED_REQUEST'
const now = Math.floor(Date.now() / 1000)

function trustLevel(level: number): TrustLevelInfo {
  return {
    level,
    automatic_level: level,
    override_level: null,
    paid_amount: level === 0 ? 0 : 50,
    discount_ratio: level > 0 ? 0.98 : 1,
    discount_percent: level > 0 ? 2 : 0,
    next_level: level < 2 ? level + 1 : null,
    next_level_paid_amount: level < 2 ? 100 : null,
    amount_to_next_level: level < 2 ? 50 : null,
    inactivity_decay_steps: 0,
    decay_period_days: 90,
    overridden: false,
  }
}

const DEBUG_USERS: Record<DebugPersonaId, AuthUser> = {
  l0: {
    id: 1001,
    username: 'debug_l0_newcomer',
    display_name: 'L0 Newcomer',
    role: ROLE.USER,
    status: 1,
    group: 'default',
    quota: 0,
    used_quota: 0,
    request_count: 0,
    developer_access_granted: false,
    trust_level_info: trustLevel(0),
    onboarding: {
      activation_complete: false,
      credential_complete: false,
      first_request_complete: false,
      stage: 'activate',
    },
  },
  l1: {
    id: 1002,
    username: 'debug_l1_developer',
    display_name: 'L1 Developer',
    role: ROLE.USER,
    status: 1,
    group: 'default',
    quota: 500_000,
    used_quota: 125_000,
    request_count: 42,
    developer_access_granted: true,
    trust_level_info: trustLevel(1),
    onboarding: {
      activation_complete: true,
      credential_complete: true,
      first_request_complete: true,
      stage: 'complete',
    },
  },
  admin: {
    id: 1099,
    username: 'debug_administrator',
    display_name: 'Administrator',
    role: ROLE.SUPER_ADMIN,
    status: 1,
    group: 'admin',
    quota: 10_000_000,
    used_quota: 2_500_000,
    request_count: 390,
    developer_access_granted: true,
    trust_level_info: trustLevel(4),
    onboarding: {
      activation_complete: true,
      credential_complete: true,
      first_request_complete: true,
      stage: 'complete',
    },
  },
}

function initialConversations(): MockConversation[] {
  return [
    {
      id: 8101,
      userId: DEBUG_USERS.l0.id,
      title: 'Need help requesting L1 access',
      preview: 'My email is [REDACTED:EMAIL] and I need L1 access.',
      createdAt: now - 3_600,
      messages: [
        {
          id: 9101,
          role: 'user',
          content: 'My email is [REDACTED:EMAIL] and I need L1 access.',
          created_at: now - 3_600,
        },
        {
          id: 9102,
          role: 'assistant',
          content:
            'Tell me what you want to build. I can help prepare an access request.',
          created_at: now - 3_540,
        },
      ],
    },
    {
      id: 8102,
      userId: DEBUG_USERS.l1.id,
      title: 'SDK configuration help',
      preview: 'Show me the OpenAI-compatible client setup.',
      createdAt: now - 7_200,
      messages: [
        {
          id: 9201,
          role: 'user',
          content: 'Show me the OpenAI-compatible client setup.',
          created_at: now - 7_200,
        },
        {
          id: 9202,
          role: 'assistant',
          content: 'Open the setup guide and select your client platform.',
          created_at: now - 7_140,
        },
      ],
    },
  ]
}

let state: DebugState = {
  activePersona: 'l0',
  conversations: initialConversations(),
}

function installBlockedDebugFetch(): void {
  const originalFetch = globalThis.fetch.bind(globalThis)
  globalThis.fetch = async (input, init) => {
    const request = input instanceof Request ? input : new Request(input, init)
    const url = new URL(request.url, window.location.origin)
    if (
      url.origin === window.location.origin &&
      url.pathname === '/api/status'
    ) {
      return new Response(
        JSON.stringify(
          envelope({
            system_name: 'LMM Persona Lab',
            logo: '/logo.png',
            assistant: { enabled: true },
            announcements_enabled: false,
          })
        ),
        { status: 200, headers: { 'content-type': 'application/json' } }
      )
    }
    if (url.origin !== window.location.origin) {
      throw new Error(`PERSONA_DEBUG_EXTERNAL_REQUEST: ${url.origin}`)
    }
    if (isBackendPath(url.pathname)) {
      throw new Error(
        `${BLOCKED_DEBUG_REQUEST}: ${request.method} ${url.pathname}`
      )
    }
    return originalFetch(request)
  }
}

function isBackendPath(pathname: string): boolean {
  return ['/api', '/mj', '/pg'].some(
    (prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`)
  )
}

function cloneUser(persona: DebugPersonaId): AuthUser {
  return structuredClone(DEBUG_USERS[persona])
}

function authBundle(persona: DebugPersonaId): AuthBundle {
  const issuedAt = Math.floor(Date.now() / 1000)
  return {
    access_token: `debug-persona-${persona}`,
    token_type: 'Bearer',
    access_expires_at: issuedAt + 86_400,
    user: cloneUser(persona),
    session: {
      sid: `debug-persona-${persona}`,
      current: true,
      login_method: 'development fixture',
      ip: '127.0.0.1',
      user_agent: 'LMM persona debug runtime',
      created_at: issuedAt,
      last_active_at: issuedAt,
      expires_at: issuedAt + 86_400,
    },
  }
}

function envelope<T>(data: T) {
  return { success: true, data }
}

function response<T>(
  config: InternalAxiosRequestConfig,
  data: T,
  status = 200
): AxiosResponse<T> {
  return {
    data,
    status,
    statusText: status === 200 ? 'OK' : 'Mock response',
    headers: {},
    config,
  }
}

function requestPath(config: InternalAxiosRequestConfig): URL {
  return new URL(config.url ?? '/', window.location.origin)
}

function activeUser(): AuthUser {
  return cloneUser(state.activePersona)
}

function conversationSummary(conversation: MockConversation) {
  return {
    id: conversation.id,
    title: conversation.title,
    last_message_preview: conversation.preview,
    created_at: conversation.createdAt,
    updated_at: conversation.createdAt + 60,
    archived_at: 0,
    owner:
      conversation.userId === activeUser().id ? 'self' : 'lower_level_user',
    privacy_notice:
      'Debug fixture: higher-access users can review lower-access conversations.',
  }
}

function parseRequestBody(config: InternalAxiosRequestConfig): unknown {
  if (typeof config.data !== 'string') return config.data
  try {
    return JSON.parse(config.data)
  } catch {
    return config.data
  }
}

function assistantReply(content: string) {
  return {
    choices: [{ message: { content } }],
  }
}

function readAuditUserId(config: InternalAxiosRequestConfig, url: URL): number {
  const raw =
    (config.params as { user_id?: unknown } | undefined)?.user_id ??
    url.searchParams.get('user_id')
  const id = Number(raw)
  return Number.isSafeInteger(id) && id > 0 ? id : activeUser().id
}

function userById(id: number): AuthUser | undefined {
  return Object.values(DEBUG_USERS).find((user) => user.id === id)
}

function canReadUserConversations(targetUserId: number): boolean {
  const viewer = activeUser()
  if (viewer.id === targetUserId) return true
  const target = userById(targetUserId)
  if (!target) return false
  if (viewer.role >= ROLE.ADMIN && viewer.role > target.role) return true
  return (
    (viewer.trust_level_info?.level ?? 0) >
    (target.trust_level_info?.level ?? 0)
  )
}

function rejectRequest(
  config: InternalAxiosRequestConfig,
  status: number,
  message: string
): never {
  const rejectedResponse = response(config, { success: false, message }, status)
  throw new AxiosError(
    message,
    AxiosError.ERR_BAD_REQUEST,
    config,
    undefined,
    rejectedResponse
  )
}

const debugAdapter: AxiosAdapter = async (config) => {
  const url = requestPath(config)
  const path = url.pathname
  const method = (config.method ?? 'get').toUpperCase()

  if (method === 'GET' && path === '/api/status') {
    return response(
      config,
      envelope({
        system_name: 'LMM Persona Lab',
        logo: '/logo.png',
        assistant: { enabled: true },
        announcements_enabled: false,
        registration_enabled: false,
        email_verification: false,
        turnstile_check: false,
      })
    )
  }
  if (method === 'GET' && path === '/api/setup') {
    return response(config, envelope({ status: true }))
  }
  if (method === 'POST' && path === '/api/user/auth/refresh') {
    return response(config, envelope(authBundle(state.activePersona)))
  }
  if (method === 'GET' && path === '/api/user/self') {
    return response(config, envelope(activeUser()))
  }
  if (method === 'GET' && path === '/api/notice') {
    return response(config, envelope(''))
  }
  if (method === 'GET' && path === '/api/release-notes/latest') {
    return response(config, envelope(null))
  }
  if (method === 'GET' && path === '/api/user/models') {
    return response(config, envelope(['gpt-5-mini', 'claude-sonnet']))
  }
  if (method === 'GET' && path === '/api/user/self/groups') {
    return response(
      config,
      envelope({ default: { desc: 'Default', ratio: 1 } })
    )
  }
  if (method === 'GET' && path === '/api/uptime/status') {
    return response(config, envelope([]))
  }
  if (method === 'GET' && path === '/api/token/') {
    return response(
      config,
      state.activePersona === 'l0'
        ? { success: false, message: 'Developer access required' }
        : {
            success: true,
            data: { items: [], total: 0, page: 1, page_size: 10 },
          },
      state.activePersona === 'l0' ? 403 : 200
    )
  }
  if (method === 'GET' && (path === '/api/data/self' || path === '/api/data')) {
    return response(config, envelope([]))
  }
  if (method === 'GET' && path === '/api/perf-metrics/summary') {
    return response(config, envelope({ models: [] }))
  }
  if (method === 'GET' && path === '/api/assistant/status') {
    const user = activeUser()
    return response(
      config,
      envelope({
        enabled: true,
        model: 'debug-fixture',
        developer_access_granted: user.developer_access_granted === true,
        funding: { mode: 'super_administrator' },
        trust_level: user.trust_level_info?.level ?? 0,
        role: user.role,
        is_admin: user.role >= ROLE.ADMIN,
        is_root: user.role === ROLE.SUPER_ADMIN,
        capabilities: {
          public_assistant: true,
          account: true,
          developer_tools: user.developer_access_granted === true,
          admin_config: user.role >= ROLE.ADMIN,
        },
      })
    )
  }
  if (method === 'POST' && path === '/api/assistant/chat') {
    const body = parseRequestBody(config) as { message?: string }
    return response(
      config,
      assistantReply(
        `Debug assistant received: ${String(body?.message ?? '').trim()}`
      )
    )
  }
  if (method === 'GET' && path === '/api/assistant/conversations') {
    const requestedUserId = readAuditUserId(config, url)
    if (!canReadUserConversations(requestedUserId)) {
      rejectRequest(config, 404, 'Conversation history is unavailable')
    }
    const visible = state.conversations.filter(
      (conversation) => conversation.userId === requestedUserId
    )
    return response(
      config,
      envelope({
        conversations: visible.map(conversationSummary),
        privacy_notice: 'This is non-production fixture data.',
      })
    )
  }
  const detail = path.match(/^\/api\/assistant\/conversations\/(\d+)$/)
  if (method === 'GET' && detail) {
    const conversation = state.conversations.find(
      (item) => item.id === Number(detail[1])
    )
    if (conversation) {
      if (!canReadUserConversations(conversation.userId)) {
        rejectRequest(config, 404, 'Conversation history is unavailable')
      }
      return response(
        config,
        envelope({
          conversation: conversationSummary(conversation),
          messages: conversation.messages,
          privacy_notice: 'This is non-production fixture data.',
        })
      )
    }
    rejectRequest(config, 404, 'Conversation history is unavailable')
  }
  if (method === 'GET' && path === '/api/assistant/handoffs/self') {
    return response(config, envelope(null))
  }
  if (method === 'GET' && path === '/api/user/self/onboarding/todo') {
    const granted = activeUser().developer_access_granted === true
    return response(
      config,
      envelope({
        eligibility: {
          eligible: granted,
          developer_access_granted: granted,
          trust_level: activeUser().trust_level_info?.level ?? 0,
        },
        status: granted ? 'completed' : 'unavailable',
        steps: [],
      })
    )
  }
  if (method === 'GET' && path === '/api/user/developer-access/request') {
    return response(config, envelope(null))
  }

  throw new Error(`${BLOCKED_DEBUG_REQUEST}: ${method} ${path}`)
}

export function installPersonaDebugRuntime(): void {
  api.defaults.adapter = debugAdapter
  setDevelopmentAuthRefreshAdapter(debugAdapter)
  installBlockedDebugFetch()
  applyAuthBundle(authBundle(state.activePersona), false)
  document.documentElement.dataset.personaDebug = 'true'
}

export function getActiveDebugPersona(): DebugPersonaId {
  return state.activePersona
}

export function setActiveDebugPersona(persona: DebugPersonaId): void {
  state = { ...state, activePersona: persona }
  applyAuthBundle(authBundle(persona), false)
  window.dispatchEvent(
    new CustomEvent<DebugEvent>(DEBUG_EVENT, { detail: { persona } })
  )
}

export function resetPersonaDebugRuntime(): void {
  state = { activePersona: 'l0', conversations: initialConversations() }
  setActiveDebugPersona('l0')
}

export function subscribeDebugPersona(
  listener: (persona: DebugPersonaId) => void
): () => void {
  const handleChange = (event: Event) => {
    listener((event as CustomEvent<DebugEvent>).detail.persona)
  }
  window.addEventListener(DEBUG_EVENT, handleChange)
  return () => window.removeEventListener(DEBUG_EVENT, handleChange)
}
