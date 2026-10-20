/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { UsageLog } from '../data/schema'
import type { LogOtherData } from '../types'

type ReceiptStateLog = Pick<UsageLog, 'billing_basis' | 'state'>
type ReceiptStateMetadata = Pick<
  LogOtherData,
  'billing_basis' | 'request_state'
>
type IncompleteReceiptState =
  | { status: 'unknown'; label: 'Unknown'; variant: 'warning' }
  | { status: 'incomplete'; label: 'Incomplete'; variant: 'warning' }

/** Native incomplete receipts retain their state without changing legacy log types. */
export function usageReceiptState(
  log: ReceiptStateLog,
  other: ReceiptStateMetadata | null = null
): IncompleteReceiptState | undefined {
  if (
    log.billing_basis !== 'incomplete' &&
    other?.billing_basis !== 'incomplete'
  ) {
    return undefined
  }
  return (log.state ?? other?.request_state) === 'reconcile'
    ? { status: 'unknown', label: 'Unknown', variant: 'warning' }
    : { status: 'incomplete', label: 'Incomplete', variant: 'warning' }
}
