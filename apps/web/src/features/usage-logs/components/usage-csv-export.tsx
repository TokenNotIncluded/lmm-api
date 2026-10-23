/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  createTrustedObjectUrl,
  revokeTrustedObjectUrl,
} from '@/lib/validated-external-url'

import {
  readUsageCsv,
  UsageCsvExportError,
  type CsvExportFailure,
} from '../lib/csv-export'
import type { UsageStatisticsScope } from '../lib/statistics'
import type { GetLogsParams } from '../types'

const failureMessages: Record<CsvExportFailure, string> = {
  invalid: 'Choose an export time range of at most 31 days.',
  too_dense: 'Too many logs to export. Narrow the time range or filters.',
  busy: 'Another log export is running. Try again after it finishes.',
  timeout: 'Log export timed out. Narrow the time range and try again.',
  forbidden: 'Your session cannot export these logs. Sign in and try again.',
  unavailable: 'Log export is unavailable. No file was downloaded.',
  session_changed: 'Your session changed. Start the log export again.',
  cancelled: 'Log export cancelled.',
  incomplete: 'The log export was incomplete. No file was downloaded.',
}

export function UsageCsvExport({
  params,
  isAdmin,
  scope,
}: {
  params: GetLogsParams
  isAdmin: boolean
  scope: UsageStatisticsScope | undefined
}) {
  const { t } = useTranslation()
  const active = useRef<AbortController | null>(null)
  const key = JSON.stringify([
    params,
    isAdmin,
    scope?.userId,
    scope?.sessionId,
    scope?.role,
  ])
  const [state, setState] = useState<{
    key: string
    busy: boolean
    message?: string
    count?: number
  }>({ key, busy: false })
  const current = state.key === key ? state : { key, busy: false }
  useEffect(
    () => () => {
      active.current?.abort()
      active.current = null
    },
    [key]
  )

  const start = async () => {
    if (!scope || active.current) return
    const controller = new AbortController()
    active.current = controller
    setState({ key, busy: true })
    try {
      const document = await readUsageCsv({
        params,
        isAdmin,
        scope,
        signal: controller.signal,
      })
      if (active.current !== controller || controller.signal.aborted) return
      const url = createTrustedObjectUrl(document.blob)
      try {
        const link = window.document.createElement('a')
        link.href = url
        link.download = document.filename
        window.document.body.append(link)
        link.click()
        link.remove()
      } finally {
        setTimeout(() => revokeTrustedObjectUrl(url), 1000)
      }
      setState({
        key,
        busy: false,
        message: 'CSV downloaded. Matching logs: {{count}}.',
        count: document.rows,
      })
    } catch (error) {
      if (active.current !== controller) return
      const reason =
        error instanceof UsageCsvExportError ? error.reason : 'unavailable'
      setState({ key, busy: false, message: failureMessages[reason] })
    } finally {
      if (active.current === controller) active.current = null
    }
  }

  return (
    <div className='flex min-w-0 flex-wrap items-center gap-2'>
      <Button
        type='button'
        variant='outline'
        size='sm'
        disabled={!scope || current.busy}
        onClick={() => void start()}
        title={t(
          'Export every log matching the applied filters, up to 10,000 logs and 31 days.'
        )}
      >
        {current.busy ? t('Exporting CSV...') : t('Export CSV')}
      </Button>
      {current.busy && (
        <Button
          type='button'
          variant='ghost'
          size='sm'
          onClick={() => active.current?.abort()}
        >
          {t('Cancel')}
        </Button>
      )}
      {current.message && (
        <p role='status' className='text-muted-foreground w-full text-xs'>
          {t(current.message, { count: current.count })}
        </p>
      )}
    </div>
  )
}
