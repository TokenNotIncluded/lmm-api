import { useQueryClient } from '@tanstack/react-query'
// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import { useEffect, useState, useSyncExternalStore } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { Dialog } from '@/components/dialog'
import { Button } from '@/components/ui/button'
import { refreshCurrentAccount } from '@/features/onboarding/use-auth-user-refresh'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  activateBountyAccount,
  bountyOperationsServerSnapshot,
  bountyOperationsSnapshot,
  completeBountyOperation,
  pendingBountyOperation,
  settleBountyOperation,
  trustedBountyConfirmationURL,
  receiptSchema,
  subscribeBountyOperations,
  updateBountyOperation,
} from './native-operation'

const envelope = z.object({ success: z.boolean(), data: z.unknown() })
export function BountyOperationDialog() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const user = useAuthStore((state) => state.auth.user)
  const operations = useSyncExternalStore(
    subscribeBountyOperations,
    bountyOperationsSnapshot,
    bountyOperationsServerSnapshot
  )
  const operation = operations[0]
  const [open, setOpen] = useState(true)
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const accountId = user ? String(user.id) : 'session'
  const operationId = operation?.id
  useEffect(() => {
    activateBountyAccount(accountId)
  }, [accountId])
  useEffect(() => {
    if (operationId) {
      setOpen(true)
      setFailed(false)
    }
  }, [operationId])
  if (!operation) return null
  const base = `/api/open-source-bounties/operations/${encodeURIComponent(operation.id)}`
  const finished = async () => {
    const hadWaiter = settleBountyOperation(operation.id)
    if (!hadWaiter) {
      await queryClient.invalidateQueries({
        queryKey: ['open-source-bounties'],
      })
      await refreshCurrentAccount()
    }
  }
  const request = async (action: 'apply' | 'recover') => {
    const response = await api.post<unknown>(`${base}/${action}`, undefined, {
      skipErrorHandler: true,
      skipBusinessError: true,
    })
    const result = envelope.parse(response.data)
    if (!result.success) throw new Error('Bounty operation failed')
    if (response.status === 200) {
      await finished()
      return
    }
    const id = pendingBountyOperation(result.data, operation.id)
    if (id !== operation.id) throw new Error('Bounty operation mismatch')
  }
  const run = async (action: 'check' | 'apply' | 'recover') => {
    setBusy(true)
    setFailed(false)
    try {
      if (action !== 'check') {
        await request(action)
        return
      }
      const response = await api.get<unknown>(base, {
        skipErrorHandler: true,
        skipBusinessError: true,
      })
      const result = envelope.parse(response.data)
      if (!result.success) throw new Error('Bounty operation unavailable')
      const receipt = receiptSchema.parse(result.data)
      if (receipt.operation_id !== operation.id) {
        throw new Error('Bounty operation mismatch')
      }
      switch (receipt.state) {
        case 'awaiting_confirmation':
          updateBountyOperation(operation.id, {
            phase: operation.phase === 'unknown' ? 'unknown' : 'confirmation',
            url: receipt.confirmation_url ?? operation.url,
          })
          break
        case 'confirmed':
          updateBountyOperation(operation.id, {
            phase: operation.phase === 'unknown' ? 'unknown' : 'confirmed',
            url: receipt.confirmation_url ?? operation.url,
          })
          break
        case 'confirmation_expired':
          updateBountyOperation(operation.id, { phase: 'expired' })
          break
        default:
          // A Core receipt alone is not a restored Go business projection.
          await request('recover')
      }
    } catch {
      setFailed(true)
      updateBountyOperation(operation.id, { phase: 'unknown' })
    } finally {
      setBusy(false)
    }
  }
  const confirmationURL = trustedBountyConfirmationURL(
    operation.url,
    window.location.origin,
    import.meta.env?.VITE_CORE_PUBLIC_ORIGIN
  )
  return (
    <>
      {!open && (
        <div className='border-border bg-card mx-4 my-2 flex flex-wrap items-center justify-between gap-2 rounded-lg border p-3 text-sm'>
          <span>{t('Pending bounty action')}</span>
          <Button size='sm' variant='outline' onClick={() => setOpen(true)}>
            {t('Continue')}
          </Button>
        </div>
      )}
      <Dialog
        open={open}
        onOpenChange={setOpen}
        title={t('Confirm bounty action')}
        description={t(
          'Review and confirm this exact action on the confirmation page, then continue here.'
        )}
      >
        <div className='space-y-3 text-sm'>
          <p className='text-muted-foreground break-all'>
            {t('Bounty operation ID')}: <code>{operation.id}</code>
          </p>
          <p role='status'>
            {operation.phase === 'unknown' || failed
              ? t(
                  'Result unknown. Check and recover this operation before starting another action.'
                )
              : operation.phase === 'expired'
                ? t(
                    'This confirmation expired. Close it before starting a new action.'
                  )
                : operation.phase === 'confirmed'
                  ? t('Apply reviewed change')
                  : t('Awaiting confirmation')}
          </p>
          <div className='flex flex-wrap gap-2'>
            {confirmationURL && operation.phase !== 'expired' && (
              <Button
                render={
                  <a
                    href={confirmationURL}
                    target='_blank'
                    rel='noopener noreferrer'
                  />
                }
              >
                {t('Open bounty confirmation page')}
              </Button>
            )}
            {operation.phase === 'unknown' ? (
              <Button disabled={busy} onClick={() => void run('recover')}>
                {t('Check and recover')}
              </Button>
            ) : operation.phase === 'confirmed' ? (
              <Button disabled={busy} onClick={() => void run('apply')}>
                {t('Apply')}
              </Button>
            ) : (
              operation.phase !== 'expired' && (
                <Button
                  variant='outline'
                  disabled={busy || operation.phase === 'preparing'}
                  onClick={() => void run('check')}
                >
                  {t('Refresh')}
                </Button>
              )
            )}
            {operation.phase === 'expired' && (
              <Button
                variant='outline'
                onClick={() =>
                  completeBountyOperation(
                    operation.id,
                    new Error('Confirmation expired')
                  )
                }
              >
                {t('Close')}
              </Button>
            )}
          </div>
        </div>
      </Dialog>
    </>
  )
}
