/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import { z } from 'zod'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  catalogSchema,
  previewSchema,
  publishedSchema,
  type Adjustment,
  type Expected,
  type Preview,
  type PriorityChange,
} from './contract'

export type Scope = {
  userId: number
  sessionId: string | undefined
  role: number
}
export function currentScope(): Scope | undefined {
  const { user, session } = useAuthStore.getState().auth
  return user && Number.isSafeInteger(user.id) && user.id > 0
    ? { userId: user.id, sessionId: session?.sid, role: user.role }
    : undefined
}
export class RoutingAdminError extends Error {
  constructor(readonly code: string) {
    super(code)
  }
}
function assertScope(scope: Scope, signal?: AbortSignal) {
  const now = currentScope()
  if (signal?.aborted) throw new RoutingAdminError('cancelled')
  if (
    now?.userId !== scope.userId ||
    now?.sessionId !== scope.sessionId ||
    now?.role !== scope.role
  )
    throw new RoutingAdminError('session_changed')
}
const base = '/api/core/routing-admin'
async function read<T>(
  scope: Scope,
  path: string,
  schema: z.ZodType<T>,
  body?: unknown,
  signal?: AbortSignal
): Promise<T> {
  assertScope(scope, signal)
  try {
    const options = {
      authScope: scope,
      signal,
      timeout: 15000,
      skipErrorHandler: true,
      skipBusinessError: true,
      skipAuthRefresh: true,
      disableDuplicate: true,
    }
    const response =
      body === undefined
        ? await api.get(base + path, options)
        : await api.post(base + path, body, options)
    assertScope(scope, signal)
    const parsed = z
      .object({ success: z.literal(true), data: schema })
      .parse(response.data)
    return parsed.data
  } catch (error) {
    assertScope(scope, signal)
    if (error instanceof RoutingAdminError) throw error
    const code =
      isAxiosError(error) && typeof error.response?.data?.code === 'string'
        ? error.response.data.code
        : 'routing_admin_unavailable'
    throw new RoutingAdminError(code)
  }
}
export function getCatalog(scope: Scope, signal?: AbortSignal) {
  return read(scope, '/catalog', catalogSchema, undefined, signal)
}
export function pricePreview(
  scope: Scope,
  expected: Expected,
  source_json: string,
  selections: { model: string; adjustment: Adjustment }[],
  signal?: AbortSignal
) {
  return read(
    scope,
    '/prices/preview',
    previewSchema,
    { expected, source_json, selections },
    signal
  )
}
export function priorityPreview(
  scope: Scope,
  expected: Expected,
  route_id: string,
  changes: PriorityChange[],
  signal?: AbortSignal
) {
  return read(
    scope,
    '/priorities/preview',
    previewSchema,
    { expected, route_id, changes },
    signal
  )
}
export function undoPreview(
  scope: Scope,
  expected: Expected,
  operation_id: string,
  signal?: AbortSignal
) {
  return read(
    scope,
    '/priorities/undo-preview',
    previewSchema,
    { expected, operation_id },
    signal
  )
}
export function operationId() {
  const bytes = new Uint8Array(32)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
}
export async function publish(
  scope: Scope,
  preview: Preview,
  operation_id: string
) {
  const result = await read(scope, '/commit', publishedSchema, {
    preview_id: preview.preview_id,
    confirmation_sha256: preview.confirmation_sha256,
    operation_id,
  })
  if (
    result.operation_id !== operation_id ||
    result.published.routing !== preview.next.routing ||
    result.published.pricing !== preview.next.pricing ||
    result.published.snapshot_sha256 !== preview.next.snapshot_sha256
  )
    throw new RoutingAdminError('routing_admin_unavailable')
  return result
}
export async function exportPrices(scope: Scope): Promise<Blob> {
  assertScope(scope)
  const response = await api.get(base + '/prices/export', {
    authScope: scope,
    timeout: 15000,
    responseType: 'blob',
    skipAuthRefresh: true,
    skipErrorHandler: true,
    disableDuplicate: true,
  })
  assertScope(scope)
  if (
    !(response.data instanceof Blob) ||
    response.data.size > 16 * 1024 * 1024 ||
    !String(response.headers['content-type']).startsWith('application/json')
  )
    throw new RoutingAdminError('routing_admin_unavailable')
  return response.data
}
export async function readSourceFile(file: File): Promise<string> {
  if (file.size > 16 * 1024 * 1024)
    throw new RoutingAdminError('routing_admin_too_large')
  const text = await file.text()
  if (new TextEncoder().encode(text).byteLength > 16 * 1024 * 1024)
    throw new RoutingAdminError('routing_admin_too_large')
  return text
}
export function download(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob)
  try {
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = filename
    anchor.click()
  } finally {
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  }
}
export const errorMessages: Record<string, string> = {
  routing_admin_conflict:
    'Routing or prices changed. Reload and create a new preview.',
  routing_admin_preview_expired: 'This preview expired. Create a new preview.',
  routing_admin_forbidden: 'You do not have permission to publish this change.',
  routing_admin_unauthorized: 'Sign in again to continue.',
  routing_admin_invalid_source:
    'Use a valid Core native price source v1 file in USD.',
  routing_admin_unsupported_price:
    'The source must preserve the current pricing and metering contract.',
  routing_admin_inexact_exchange:
    'This exchange adjustment cannot be represented exactly. No prices were changed.',
  routing_admin_exchange_unavailable:
    'A valid site exchange rate is required for this adjustment.',
  routing_admin_invalid: 'No valid changes were selected.',
  routing_admin_too_large: 'The import or preview exceeds the supported limit.',
  routing_admin_busy: 'Too many active previews. Wait for a preview to expire.',
  session_changed: 'Your session changed. Reload before continuing.',
  cancelled: 'Cancelled',
  routing_admin_unavailable:
    'Routing administration is unavailable. Retry when the service is ready.',
}
export function errorMessage(error: unknown) {
  return error instanceof RoutingAdminError
    ? (errorMessages[error.code] ?? errorMessages.routing_admin_unavailable)
    : errorMessages.routing_admin_unavailable
}
