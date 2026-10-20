// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import { z } from 'zod'

const operationId = z.string().regex(/^[A-Za-z0-9_.:-]{1,128}$/)
const confirmationUrl = z
  .string()
  .regex(/^\/core\/v1\/bounties\/confirm\/[A-Za-z0-9_-]{1,128}$/)
const pendingSchema = z.object({
  operation_id: operationId,
  state: z.enum([
    'awaiting_confirmation',
    'confirmed',
    'confirmation_expired',
    'preparing',
    'dispatching',
    'unknown',
  ]),
  confirmation_url: z.union([confirmationUrl, z.literal('')]).optional(),
})
const savedSchema = z.object({
  id: operationId,
  url: z.union([confirmationUrl, z.literal('')]),
  phase: z.enum([
    'preparing',
    'confirmation',
    'confirmed',
    'unknown',
    'expired',
  ]),
})
export type NativeBountyOperation = z.infer<typeof savedSchema>
export const receiptSchema = z.object({
  operation_id: operationId,
  state: z.string().min(1),
  confirmation_url: z.union([confirmationUrl, z.literal('')]).optional(),
})
let account = ''
const empty: NativeBountyOperation[] = []
let snapshot = empty
const subscribers = new Set<() => void>()
const waiters = new Map<
  string,
  Array<{ resolve: () => void; reject: (error: Error) => void }>
>()
const storageKey = () => `lmm:bounty-operations:${account}`
function publish(next: NativeBountyOperation[]) {
  snapshot = next
  try {
    sessionStorage.setItem(storageKey(), JSON.stringify(next))
  } catch {
    /* The server retains canonical arguments; keep the ID in memory. */
  }
  for (const notify of subscribers) notify()
}
export function activateBountyAccount(id: string) {
  if (account === id) return
  for (const values of waiters.values()) {
    for (const waiter of values) waiter.reject(new Error('Session changed'))
  }
  waiters.clear()
  account = id
  let restored: NativeBountyOperation[] = []
  try {
    const parsed = savedSchema
      .array()
      .safeParse(JSON.parse(sessionStorage.getItem(storageKey()) ?? '[]'))
    if (parsed.success) {
      restored = parsed.data.map((op) => ({
        ...op,
        phase: op.phase === 'preparing' ? 'unknown' : op.phase,
      }))
    }
  } catch {
    /* No browser storage is available in server rendering. */
  }
  publish(restored)
}
export function subscribeBountyOperations(notify: () => void) {
  subscribers.add(notify)
  return () => subscribers.delete(notify)
}
export function bountyOperationsSnapshot() {
  return snapshot
}
export function bountyOperationsServerSnapshot() {
  return empty
}
export function beginBountyOperation(id: string) {
  operationId.parse(id)
  const existing = snapshot.find((op) => op.id === id)
  if (existing) return false
  if (snapshot.length > 0) {
    throw new Error('A bounty action is already pending.')
  }
  publish([{ id, url: '', phase: 'preparing' }])
  return true
}
export function pendingBountyOperation(value: unknown, expectedID?: string) {
  const pending = pendingSchema.parse(value)
  if (expectedID && pending.operation_id !== expectedID) {
    throw new Error('Bounty operation mismatch')
  }
  const phase =
    pending.state === 'awaiting_confirmation'
      ? 'confirmation'
      : pending.state === 'confirmed'
        ? 'confirmed'
        : pending.state === 'confirmation_expired'
          ? 'expired'
          : 'unknown'
  updateBountyOperation(pending.operation_id, {
    url: pending.confirmation_url ?? '',
    phase,
  })
  return pending.operation_id
}
export function updateBountyOperation(
  id: string,
  value: Pick<NativeBountyOperation, 'phase'> & { url?: string }
) {
  const found = snapshot.find((op) => op.id === id)
  if (!found) return
  publish(snapshot.map((op) => (op.id === id ? { ...op, ...value } : op)))
}
export function waitForBountyOperation(id: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const values = waiters.get(id) ?? []
    values.push({ resolve, reject })
    waiters.set(id, values)
  })
}
export function settleBountyOperation(id: string) {
  const values = waiters.get(id) ?? []
  waiters.delete(id)
  for (const waiter of values) waiter.resolve()
  if (values.length === 0) completeBountyOperation(id)
  return values.length > 0
}
export function hasBountyOperation(id: string) {
  return snapshot.some((op) => op.id === id)
}
export function completeBountyOperation(id: string, error?: Error) {
  const values = waiters.get(id) ?? []
  waiters.delete(id)
  publish(snapshot.filter((op) => op.id !== id))
  for (const waiter of values) {
    if (error) waiter.reject(error)
    else waiter.resolve()
  }
  return values.length > 0
}

// The backend supplies only an exact confirmation path. The deployment chooses
// the Core origin; neither Host headers nor arbitrary response URLs choose it.
export function trustedBountyConfirmationURL(
  raw: string,
  webOrigin: string,
  configuredCoreOrigin?: string
): string | null {
  if (!confirmationUrl.safeParse(raw).success) return null
  const origin = configuredCoreOrigin || webOrigin
  try {
    const url = new URL(origin)
    const local =
      url.protocol === 'http:' && ['127.0.0.1', '[::1]'].includes(url.hostname)
    if (
      (!local && url.protocol !== 'https:') ||
      url.origin !== origin ||
      url.username ||
      url.password
    ) {
      return null
    }
    return new URL(raw, url.origin).toString()
  } catch {
    return null
  }
}
