/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import { Check, RefreshCw, ShieldAlert, X } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { toIntlLocale } from '@/i18n/languages'

import { listAccountActionRequests, reviewAccountActionRequest } from '../api'
import type { AccountActionRequestAdmin } from '../types'

function isNotFound(error: unknown): boolean {
  return (
    (error as { response?: { status?: number } } | null)?.response?.status ===
    404
  )
}

export function AccountActionRequestsPanel() {
  const { t, i18n } = useTranslation()
  const [requests, setRequests] = useState<AccountActionRequestAdmin[]>([])
  const [loading, setLoading] = useState(true)
  const [available, setAvailable] = useState(true)
  const [reviewing, setReviewing] = useState<number | null>(null)
  const [notes, setNotes] = useState<Record<number, string>>({})

  const loadRequests = useCallback(async () => {
    setLoading(true)
    try {
      const response = await listAccountActionRequests('pending')
      if (!response.success) {
        throw new Error(
          response.message || t('Unable to load account action requests')
        )
      }
      setRequests(response.data ?? [])
      setAvailable(true)
    } catch (error) {
      if (isNotFound(error)) {
        setAvailable(false)
      } else {
        toast.error(
          error instanceof Error
            ? error.message
            : t('Unable to load account action requests')
        )
      }
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void loadRequests()
  }, [loadRequests])

  const review = async (
    request: AccountActionRequestAdmin,
    approve: boolean
  ) => {
    if (reviewing !== null) return
    const note = (notes[request.id] ?? '').trim()
    if (!approve && [...note].length < 2) {
      toast.error(
        t(
          'A rejection reason is required and must contain at least 2 characters.'
        )
      )
      return
    }
    setReviewing(request.id)
    try {
      const response = await reviewAccountActionRequest(
        request.id,
        approve ? 'approve' : 'reject',
        note
      )
      if (!response.success) {
        throw new Error(
          response.message || t('Unable to review account action request')
        )
      }
      let successMessage = t('Account action request rejected')
      if (approve) {
        successMessage =
          request.kind === 'appeal'
            ? t('Account appeal approved')
            : t('Account disable request approved')
      }
      toast.success(successMessage)
      setRequests((current) => current.filter((item) => item.id !== request.id))
      setNotes((current) => {
        const next = { ...current }
        delete next[request.id]
        return next
      })
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to review account action request')
      )
    } finally {
      setReviewing(null)
    }
  }

  if (!available) return null

  const dateTimeFormatter = new Intl.DateTimeFormat(
    toIntlLocale(i18n.language),
    {
      dateStyle: 'medium',
      timeStyle: 'short',
    }
  )

  return (
    <section className='bg-muted/10 border px-5 py-5 sm:px-6'>
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <div>
          <div className='flex items-center gap-2'>
            <h2 className='text-sm font-semibold'>
              {t('Account safety review')}
            </h2>
            <Badge variant='secondary'>{requests.length}</Badge>
          </div>
          <p className='text-muted-foreground mt-1 text-sm'>
            {t(
              'AI can only submit disable recommendations. Account changes and appeals take effect only after administrator approval.'
            )}
          </p>
        </div>
        <Button
          variant='outline'
          size='sm'
          onClick={() => void loadRequests()}
          disabled={loading}
        >
          <RefreshCw
            data-icon='inline-start'
            className={loading ? 'animate-spin' : undefined}
          />
          {t('Refresh')}
        </Button>
      </div>

      {loading && requests.length === 0 ? (
        <p className='text-muted-foreground mt-5 text-sm'>{t('Loading...')}</p>
      ) : null}
      {!loading && requests.length === 0 ? (
        <p className='text-muted-foreground mt-5 text-sm'>
          {t('No pending account safety requests.')}
        </p>
      ) : null}

      {requests.length > 0 ? (
        <div className='mt-5 grid gap-3'>
          {requests.map((request) => {
            const isReviewing = reviewing === request.id
            const isAppeal = request.kind === 'appeal'
            return (
              <article key={request.id} className='bg-background border p-4'>
                <div className='flex flex-wrap items-start justify-between gap-3'>
                  <div className='min-w-0'>
                    <div className='flex items-center gap-2'>
                      <ShieldAlert className='size-4' aria-hidden='true' />
                      <p className='font-medium'>
                        {request.target_username || t('Unknown user')}
                      </p>
                    </div>
                    <p className='text-muted-foreground text-xs'>
                      {request.target_email || t('No email provided')} · #
                      {request.id} ·{' '}
                      {dateTimeFormatter.format(
                        new Date(request.created_at * 1000)
                      )}
                    </p>
                  </div>
                  <div className='flex flex-wrap items-center gap-2'>
                    <Badge variant={isAppeal ? 'secondary' : 'destructive'}>
                      {isAppeal
                        ? t('Account appeal')
                        : t('Disable recommendation')}
                    </Badge>
                    <Badge variant='outline'>{t('Pending review')}</Badge>
                  </div>
                </div>

                <div className='bg-muted/20 mt-3 border p-3'>
                  <p className='text-xs font-medium'>{t('Request reason')}</p>
                  <p className='text-muted-foreground mt-1 text-sm whitespace-pre-wrap'>
                    {request.reason}
                  </p>
                  <p className='text-muted-foreground mt-2 text-xs'>
                    {t('Submitted by')}:{' '}
                    {request.requested_by_username || t('Unknown user')}
                  </p>
                </div>

                <Textarea
                  className='mt-3'
                  rows={2}
                  maxLength={2000}
                  aria-label={t('Administrator review note')}
                  placeholder={t(
                    'Add an administrator note; a rejection note is required.'
                  )}
                  value={notes[request.id] ?? ''}
                  onChange={(event) =>
                    setNotes((current) => ({
                      ...current,
                      [request.id]: event.target.value,
                    }))
                  }
                />
                <div className='mt-3 flex flex-wrap justify-end gap-2'>
                  <Button
                    size='sm'
                    variant='destructive'
                    onClick={() => void review(request, false)}
                    disabled={
                      isReviewing ||
                      [...(notes[request.id] ?? '').trim()].length < 2
                    }
                  >
                    <X data-icon='inline-start' />
                    {t('Reject')}
                  </Button>
                  <Button
                    size='sm'
                    onClick={() => void review(request, true)}
                    disabled={isReviewing}
                  >
                    <Check data-icon='inline-start' />
                    {isAppeal ? t('Approve and unblock') : t('Approve disable')}
                  </Button>
                </div>
              </article>
            )
          })}
        </div>
      ) : null}
    </section>
  )
}
