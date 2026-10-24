/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import type { UsageLog } from '../data/schema'
import { cacheWrite12hQuantity, logUsageQuantity } from '../lib/usage-quantity'
import type { LogOtherData } from '../types'

export function LogUsageDisplay({
  log,
  other,
}: {
  log: UsageLog
  other: LogOtherData | null
}) {
  const { t } = useTranslation()
  const usage = logUsageQuantity(log, other)
  if (usage.kind === 'image') {
    return (
      <span>
        {t('Images (billing quantity)')}:{' '}
        {usage.count?.toLocaleString() ?? t('Unknown')}
      </span>
    )
  }
  return (
    <span>
      {usage.input?.toLocaleString() ?? '—'} /{' '}
      {usage.output?.toLocaleString() ?? '—'}
      {other && 'cache_creation_tokens_12h' in other && (
        <span className='text-muted-foreground block text-xs'>
          {t('12-hour cache writes')}:{' '}
          {cacheWrite12hQuantity(
            other,
            log.token_usage_available
          )?.toLocaleString() ?? t('Unknown')}
        </span>
      )}
    </span>
  )
}
