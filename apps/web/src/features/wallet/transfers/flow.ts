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
/*
Copyright (C) 2026 LIghtJUNction
*/
import { z } from 'zod'

export const transferSchema = z.object({
  id: z.number().int().positive().safe(),
  token: z.string().optional(),
  quota: z.number().int().positive().safe(),
  status: z.enum(['awaiting_confirmation', 'pending', 'claimed', 'cancelled']),
  created_at: z.number().int().nonnegative(),
  claimed_at: z.number().int().nonnegative().optional().default(0),
  cancelled_at: z.number().int().nonnegative().optional().default(0),
  request_key: z.string().optional(),
  confirmation_url: z.string().optional(),
  claim_link_available: z.boolean().optional(),
  recipient_details_available: z.boolean().optional(),
  recipient_id: z.number().int().positive().safe().optional(),
  recipient_username: z.string().optional(),
  recipient_name: z.string().optional(),
  recipient_email: z.string().optional(),
})
export type WalletTransfer = z.infer<typeof transferSchema>
export const receiptSchema = z.object({
  quota: z.number().int().positive().safe(),
  status: z.enum(['pending', 'claimed', 'cancelled']),
  created_at: z.number().int().nonnegative(),
  claimed_at: z.number().int().nonnegative().optional().default(0),
  is_sender: z.boolean(),
  claimed_by_me: z.boolean(),
})
export type TransferReceipt = z.infer<typeof receiptSchema>
export const envelopeSchema = z.object({
  success: z.boolean(),
  message: z.string().optional(),
  code: z.string().optional(),
  data: z.unknown().optional(),
})
export const confirmationSchema = z.object({
  confirmation_url: z.string(),
  request_key: z.string(),
  transfer: transferSchema,
})
export type TransferActionResult =
  | { kind: 'transfer'; transfer: WalletTransfer }
  | { kind: 'confirmation'; confirmationURL: string; transfer: WalletTransfer }
export type CancelActionResult =
  | { kind: 'cancelled' }
  | Extract<TransferActionResult, { kind: 'confirmation' }>
const stepSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('unknown') }),
  z.object({
    kind: z.literal('confirmation'),
    url: z.string(),
    id: z.number().int().positive().safe(),
  }),
])
export const pendingSchema = z.discriminatedUnion('action', [
  z.object({
    action: z.literal('create'),
    requestKey: z.string().min(16).max(64),
    quota: z.number().int().positive().safe(),
    step: stepSchema,
  }),
  z.object({
    action: z.literal('cancel'),
    id: z.number().int().positive().safe(),
    requestKey: z.string().optional(),
    quota: z.number().int().positive().safe(),
    step: stepSchema,
  }),
])
export type PendingTransfer = z.infer<typeof pendingSchema>
export const unknownMessage =
  'The result is unknown. Check this call instead of starting it again.'
export class WalletOutcomeUnknown extends Error {
  constructor() {
    super(unknownMessage)
  }
}
function trustedOrigin(raw: string): URL | null {
  try {
    const url = new URL(raw)
    const local =
      url.protocol === 'http:' &&
      (url.hostname === '127.0.0.1' || url.hostname === '[::1]')
    return (url.protocol === 'https:' || local) &&
      url.origin === raw &&
      !url.username &&
      !url.password
      ? url
      : null
  } catch {
    return null
  }
}
// Only a configured/same-origin Core confirmation path for this exact operation
// can open a new tab. A backend response cannot choose an arbitrary origin.
export function trustedConfirmationURL(
  raw: string,
  action: 'create' | 'cancel',
  id: number,
  webOrigin: string,
  configuredCoreOrigin?: string
): string | null {
  const origins = [webOrigin, configuredCoreOrigin].filter(
    (v): v is string => typeof v === 'string' && trustedOrigin(v) !== null
  )
  try {
    const url = new URL(raw)
    if (
      !origins.includes(url.origin) ||
      url.username ||
      url.password ||
      url.search ||
      url.hash ||
      url.pathname !== `/core/v1/wallet/confirm/${action}/${id}`
    ) {
      return null
    }
    return url.toString()
  } catch {
    return null
  }
}
export function canShareTransfer(transfer: WalletTransfer): boolean {
  return (
    transfer.status === 'pending' &&
    transfer.claim_link_available !== false &&
    typeof transfer.token === 'string' &&
    /^[a-f0-9]{64}$/.test(transfer.token)
  )
}
export function hasRecipientDetails(transfer: WalletTransfer): boolean {
  return (
    transfer.recipient_details_available !== false &&
    transfer.recipient_id !== undefined &&
    !!(
      transfer.recipient_name ||
      transfer.recipient_username ||
      transfer.recipient_email
    )
  )
}
export type ReconciledTransfer =
  | { kind: 'pending'; pending: PendingTransfer }
  | { kind: 'complete'; transfer: WalletTransfer }
  | { kind: 'conflict'; transfer: WalletTransfer }
export function reconcileTransfer(
  pending: PendingTransfer,
  result: TransferActionResult
): ReconciledTransfer {
  const transfer = result.transfer
  if (
    transfer.quota !== pending.quota ||
    (pending.action === 'cancel' && transfer.id !== pending.id) ||
    (pending.requestKey &&
      transfer.request_key &&
      pending.requestKey !== transfer.request_key)
  ) {
    throw new WalletOutcomeUnknown()
  }
  if (pending.action === 'cancel') {
    if (transfer.status === 'cancelled') return { kind: 'complete', transfer }
    if (transfer.status === 'claimed') return { kind: 'conflict', transfer }
    // Reading an existing pending transfer is not evidence of cancellation.
    return {
      kind: 'pending',
      pending:
        result.kind === 'confirmation'
          ? {
              ...pending,
              step: {
                kind: 'confirmation',
                url: result.confirmationURL,
                id: transfer.id,
              },
            }
          : pending,
    }
  }
  if (transfer.status !== 'awaiting_confirmation') {
    return { kind: 'complete', transfer }
  }
  if (result.kind !== 'confirmation') {
    return {
      kind: 'pending',
      pending: { ...pending, step: { kind: 'unknown' } },
    }
  }
  return {
    kind: 'pending',
    pending: {
      ...pending,
      step: {
        kind: 'confirmation',
        url: result.confirmationURL,
        id: transfer.id,
      },
    },
  }
}
export function readPendingTransfer(userID: number): PendingTransfer | null {
  try {
    const raw = sessionStorage.getItem(`wallet-transfer-resume:${userID}`)
    if (!raw) return null
    const parsed = pendingSchema.safeParse(JSON.parse(raw))
    return parsed.success ? parsed.data : null
  } catch {
    return null
  }
}
export function savePendingTransfer(
  userID: number,
  pending: PendingTransfer | null
): void {
  try {
    const key = `wallet-transfer-resume:${userID}`
    if (pending) sessionStorage.setItem(key, JSON.stringify(pending))
    else sessionStorage.removeItem(key)
  } catch {
    /* The in-memory operation still keeps the immutable request key. */
  }
}
