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

/*
Copyright (C) 2026 LIghtJUNction
*/
import { api } from '@/lib/api'
import { formatQuotaWithCurrency } from '@/lib/currency'

import {
  confirmationSchema,
  envelopeSchema,
  receiptSchema,
  transferSchema,
  WalletOutcomeUnknown,
  type CancelActionResult,
  type TransferActionResult,
} from './flow'

export type { WalletTransfer, TransferReceipt } from './flow'

const readConfig = { skipBusinessError: true, skipErrorHandler: true }
// An authentication retry must not dispatch a second financial POST. Recovery
// uses the immutable original key/id with GET, including after a missing ACK.
const writeConfig = { ...readConfig, skipAuthRefresh: true }
async function readResult<T>(
  request: Promise<{ data: unknown }>,
  schema: z.ZodType<T>
): Promise<T> {
  const { data } = await request
  const envelope = envelopeSchema.parse(data)
  if (!envelope.success) throw new WalletOutcomeUnknown()
  return schema.parse(envelope.data)
}
function actionResult(
  raw: unknown,
  requestKey: string | undefined,
  id: number | undefined,
  quota: number | undefined
): TransferActionResult {
  const envelope = envelopeSchema.parse(raw)
  if (!envelope.success) {
    if (envelope.code !== 'WALLET_CONFIRMATION_REQUIRED') {
      throw new WalletOutcomeUnknown()
    }
    const confirmation = confirmationSchema.parse(envelope.data)
    if (
      (requestKey && confirmation.request_key !== requestKey) ||
      (id && confirmation.transfer.id !== id) ||
      (quota && confirmation.transfer.quota !== quota)
    ) {
      throw new WalletOutcomeUnknown()
    }
    return {
      kind: 'confirmation',
      confirmationURL: confirmation.confirmation_url,
      transfer: confirmation.transfer,
    }
  }
  const transfer = transferSchema.parse(envelope.data)
  if (
    (requestKey &&
      transfer.request_key &&
      transfer.request_key !== requestKey) ||
    (id && transfer.id !== id) ||
    (quota && transfer.quota !== quota)
  ) {
    throw new WalletOutcomeUnknown()
  }
  return { kind: 'transfer', transfer }
}
export const listTransfers = (before = 0) =>
  readResult(
    api.get<unknown>('/api/wallet-transfer', {
      ...readConfig,
      params: { before },
    }),
    z.array(transferSchema)
  )
export async function createTransfer(
  quota: number,
  requestKey: string
): Promise<TransferActionResult> {
  try {
    const response = await api.post<unknown>(
      '/api/wallet-transfer',
      { quota, request_key: requestKey },
      writeConfig
    )
    return actionResult(response.data, requestKey, undefined, quota)
  } catch {
    throw new WalletOutcomeUnknown()
  }
}
export async function cancelTransfer(
  id: number,
  quota?: number
): Promise<CancelActionResult> {
  try {
    const response = await api.post<unknown>(
      `/api/wallet-transfer/${id}/cancel`,
      {},
      writeConfig
    )
    const result = actionResult(response.data, undefined, id, quota)
    if (result.kind === 'confirmation') return result
    if (result.transfer.status === 'cancelled') return { kind: 'cancelled' }
    throw new WalletOutcomeUnknown()
  } catch {
    throw new WalletOutcomeUnknown()
  }
}
export async function queryTransfer(
  requestKey: string
): Promise<TransferActionResult> {
  try {
    const response = await api.get<unknown>('/api/wallet-transfer/query', {
      ...readConfig,
      params: { request_key: requestKey },
    })
    return actionResult(response.data, requestKey, undefined, undefined)
  } catch {
    throw new WalletOutcomeUnknown()
  }
}
export async function queryTransferByID(
  id: number
): Promise<TransferActionResult> {
  try {
    const response = await api.get<unknown>('/api/wallet-transfer/query', {
      ...readConfig,
      params: { cancel_transfer_id: id },
    })
    return actionResult(response.data, undefined, id, undefined)
  } catch {
    throw new WalletOutcomeUnknown()
  }
}
export const inspectTransfer = (token: string) =>
  readResult(
    api.post<unknown>('/api/wallet-transfer/inspect', { token }, readConfig),
    receiptSchema
  )
export const claimTransfer = (token: string) =>
  readResult(
    api.post<unknown>('/api/wallet-transfer/claim', { token }, writeConfig),
    receiptSchema
  )

// A fragment is never sent in an HTTP request or Referer header. Send the
// credential to the API in a body, never in a URL or analytics event.
export const transferLink = (token: string) =>
  `${window.location.origin}/transfer#${token}`
/** Legacy batch conversion; new wallet inputs use the captured display-currency hook. */
export function transferQuota(
  amount: string,
  quotaPerUnit: number
): number | null {
  if (!/^\d+(\.\d+)?$/.test(amount)) return null
  const value = Number(amount) * quotaPerUnit
  const quota = Math.round(value)
  return Number.isSafeInteger(quota) &&
    quota > 0 &&
    Math.abs(value - quota) < 0.000001
    ? quota
    : null
}

export const formatTransferQuota = (quota: number) =>
  formatQuotaWithCurrency(quota, {
    digitsLarge: 12,
    digitsSmall: 12,
    abbreviate: false,
    compact: false,
  })
