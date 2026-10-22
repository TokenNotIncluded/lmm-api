/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

import type { Preview } from './contract'
import {
  adjustmentLabels,
  effectiveRate,
  rateLabels,
  unitLabels,
} from './helpers'

export function PublicationReview({
  preview,
  busy,
  onConfirm,
  onCancel,
}: {
  preview: Preview
  busy: boolean
  onConfirm: () => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  const expired = now >= Number(preview.expires_at) * 1000
  return (
    <section
      className='bg-card space-y-4 rounded-xl border p-4'
      aria-label={t('Review publication')}
    >
      <div>
        <h3 className='font-semibold'>{t('Review publication')}</h3>
        <p className='text-muted-foreground text-sm'>
          {t('Expires at {{time}}', {
            time: new Date(
              Number(preview.expires_at) * 1000
            ).toLocaleTimeString(),
          })}
        </p>
      </div>
      {preview.exchange_rate && (
        <p className='text-sm'>
          {t('Frozen site exchange rate')}:{' '}
          <strong>{preview.exchange_rate.raw}</strong>
        </p>
      )}
      {preview.kind === 'prices' ? (
        <>
          <p className='text-muted-foreground text-sm'>
            {t(
              'Effective rates include group and speed multipliers. Account discounts apply separately.'
            )}
          </p>
          {preview.changes.map((change) => (
            <div className='space-y-2' key={change.rule_id}>
              <h4 className='text-sm font-medium'>
                {change.model} · {change.group} · {change.speed}
              </h4>
              <p className='text-muted-foreground text-xs'>
                {t(adjustmentLabels[change.adjustment])}
              </p>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t('Usage')}</TableHead>
                    <TableHead>{t('Current price')}</TableHead>
                    <TableHead>{t('Source price')}</TableHead>
                    <TableHead>{t('Final price')}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {change.after.prices.map((rate, index) => {
                    const before = change.before.prices[index],
                      source = change.source.prices[index]
                    return (
                      <TableRow
                        key={`${rate.name}-${rate.up_to_input_tokens ?? 'all'}`}
                      >
                        <TableCell>
                          <span>{t(rateLabels[rate.name])}</span>
                          <span className='text-muted-foreground block text-xs'>
                            {t(unitLabels[rate.unit])} ·{' '}
                            {rate.up_to_input_tokens
                              ? t('Input token ceiling: {{ceiling}}', {
                                  ceiling: rate.up_to_input_tokens,
                                })
                              : t('Remaining input lengths')}
                          </span>
                        </TableCell>
                        <TableCell className='font-mono text-xs'>
                          {before ? effectiveRate(change.before, before) : '—'}
                        </TableCell>
                        <TableCell className='font-mono text-xs'>
                          {source ? effectiveRate(change.source, source) : '—'}
                        </TableCell>
                        <TableCell className='font-mono text-xs font-semibold'>
                          {effectiveRate(change.after, rate)}
                        </TableCell>
                      </TableRow>
                    )
                  })}
                </TableBody>
              </Table>
            </div>
          ))}
        </>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t('Target')}</TableHead>
              <TableHead>{t('Current priority')}</TableHead>
              <TableHead>{t('Final priority')}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {preview.changes.map((change) => (
              <TableRow key={change.target_id}>
                <TableCell>
                  #{change.target_id} · {t('Upstream')} #{change.upstream_id}
                </TableCell>
                <TableCell>{change.before}</TableCell>
                <TableCell className='font-semibold'>{change.after}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
      {expired && (
        <p role='alert' className='text-destructive text-sm'>
          {t('This preview expired. Create a new preview.')}
        </p>
      )}
      <div className='flex flex-wrap gap-2'>
        <Button disabled={busy || expired} onClick={onConfirm}>
          {busy ? t('Publishing...') : t('Confirm publication')}
        </Button>
        <Button variant='outline' disabled={busy} onClick={onCancel}>
          {t('Cancel')}
        </Button>
      </div>
    </section>
  )
}
