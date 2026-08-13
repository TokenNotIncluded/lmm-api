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
import { redactAssistantMessageForRequest } from './assistant-message-safety'

export type AssistantPresetId =
  | 'onboarding'
  | 'service'
  | 'plan'
  | 'api-key'
  | 'client-setup'
  | 'bounty'
  | 'cost'
  | 'usage'
  | 'models'
  | 'invitation'
  | 'human'

const ASSISTANT_OPEN_EVENT = 'lmm:assistant:open'
const ASSISTANT_REQUEST_QUEUE_KEY = 'lmm_assistant_request:v1'
const LEGACY_QUEUE_KEYS = [
  'lmm_assistant_queued_preset',
  'lmm_assistant_queued_message',
  'lmm_assistant_auto_send:v1',
] as const

let memoryRequest: QueuedAssistantRequest | null = null
let requestSequence = 0

export type QueuedAssistantRequest = {
  id: string
  preset?: AssistantPresetId
  message?: string
  autoSend: boolean
}

function isAssistantPresetId(value: string): value is AssistantPresetId {
  return [
    'onboarding',
    'service',
    'plan',
    'api-key',
    'client-setup',
    'bounty',
    'cost',
    'usage',
    'models',
    'invitation',
    'human',
  ].includes(value)
}

function createRequestId(): string {
  requestSequence += 1
  return `${Date.now().toString(36)}-${requestSequence.toString(36)}`
}

function clearStoredRequest(): void {
  if (typeof window === 'undefined') return
  try {
    window.sessionStorage.removeItem(ASSISTANT_REQUEST_QUEUE_KEY)
    for (const key of LEGACY_QUEUE_KEYS) window.sessionStorage.removeItem(key)
  } catch {
    // The in-memory queue still covers restricted storage environments.
  }
}

function readStoredRequest(): QueuedAssistantRequest | null {
  if (typeof window === 'undefined') return null
  try {
    const stored = window.sessionStorage.getItem(ASSISTANT_REQUEST_QUEUE_KEY)
    if (!stored) return null
    const parsed = JSON.parse(stored) as Partial<QueuedAssistantRequest>
    if (
      typeof parsed.id !== 'string' ||
      typeof parsed.autoSend !== 'boolean' ||
      (parsed.preset !== undefined &&
        !isAssistantPresetId(String(parsed.preset))) ||
      (parsed.message !== undefined && typeof parsed.message !== 'string')
    ) {
      clearStoredRequest()
      return null
    }
    return {
      id: parsed.id,
      preset: parsed.preset,
      message: parsed.message?.trim() || undefined,
      autoSend: parsed.autoSend,
    }
  } catch {
    clearStoredRequest()
    return null
  }
}

function queueAssistantRequest(
  preset?: AssistantPresetId,
  message?: string,
  autoSend = false
): void {
  const pending = memoryRequest ?? readStoredRequest()
  if (!message && !autoSend && pending?.autoSend && pending.message) {
    if (typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent<QueuedAssistantRequest>(ASSISTANT_OPEN_EVENT, {
          detail: pending,
        })
      )
    }
    return
  }
  const safeMessage = message
    ? redactAssistantMessageForRequest(message).content.trim() || undefined
    : undefined
  const request: QueuedAssistantRequest = {
    id: createRequestId(),
    preset,
    message: safeMessage,
    autoSend: autoSend && safeMessage !== undefined,
  }
  memoryRequest = request

  if (typeof window === 'undefined') return

  try {
    window.sessionStorage.setItem(
      ASSISTANT_REQUEST_QUEUE_KEY,
      JSON.stringify(request)
    )
    for (const key of LEGACY_QUEUE_KEYS) window.sessionStorage.removeItem(key)
  } catch {
    // The in-memory queue still covers restricted storage environments.
  }

  window.dispatchEvent(
    new CustomEvent<QueuedAssistantRequest>(ASSISTANT_OPEN_EVENT, {
      detail: request,
    })
  )
}

export function requestAssistantOpen(
  preset?: AssistantPresetId,
  message?: string
): void {
  queueAssistantRequest(preset, message)
}

/** Queue a one-time message that the authenticated assistant sends on mount. */
export function requestAssistantSend(
  preset: AssistantPresetId | undefined,
  message: string
): void {
  queueAssistantRequest(preset, message, true)
}

/** Consume a queued cross-route request atomically so it cannot send twice. */
export function consumeQueuedAssistantRequest(
  expectedId?: string
): QueuedAssistantRequest | undefined {
  const request = memoryRequest ?? readStoredRequest() ?? undefined
  if (!request || (expectedId && request.id !== expectedId)) return undefined
  memoryRequest = null
  clearStoredRequest()
  return request
}

export function peekQueuedAssistantRequest():
  | QueuedAssistantRequest
  | undefined {
  return memoryRequest ?? readStoredRequest() ?? undefined
}

export function subscribeToAssistantOpen(
  listener: (request: QueuedAssistantRequest) => void
): () => void {
  if (typeof window === 'undefined') return () => undefined

  const handleOpen = (event: Event) => {
    const request = (event as CustomEvent<QueuedAssistantRequest>).detail
    listener(request)
  }

  window.addEventListener(ASSISTANT_OPEN_EVENT, handleOpen)
  return () => window.removeEventListener(ASSISTANT_OPEN_EVENT, handleOpen)
}
