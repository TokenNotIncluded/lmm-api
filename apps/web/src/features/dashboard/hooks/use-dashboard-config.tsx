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
import {
  Hash,
  Coins,
  Layers,
  Gauge,
  Zap,
  Flame,
  TrendingUp,
  Activity,
  Image,
  type LucideIcon,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'

import type { IconBadgeTone } from '@/components/ui/icon-badge'
import { safeDivide } from '@/features/dashboard/lib/stats'
import type { QuotaDataItem } from '@/features/dashboard/types'

interface ModelStatValues {
  rpm: number
  quota: number | null
  tpm: number | null
  cacheRead: number | null
  cacheCreation: number | null
  outputImages: number | null
}

interface StatCardConfig {
  key: string
  title: string
  description: string
  icon: LucideIcon
  iconTone: IconBadgeTone
  getValue: (stat: ModelStatValues, minutes?: number) => number | null
}

export function useModelStatCardsConfig({
  tokenCountScope,
  includeOutputImages = false,
}: {
  tokenCountScope?: QuotaDataItem['token_count_scope']
  includeOutputImages?: boolean
} = {}): StatCardConfig[] {
  const { t } = useTranslation()
  const textOnlyTokens = tokenCountScope === 'settled_text_requests'

  const cards: StatCardConfig[] = [
    {
      key: 'count',
      title: t('Total Count'),
      description: t('Statistical count'),
      icon: Hash,
      iconTone: 'info',
      getValue: (stat) => stat?.rpm ?? 0,
    },
    {
      key: 'quota',
      title: t('Raw credits'),
      description: t('Historical monetary value unavailable'),
      icon: Coins,
      iconTone: 'success',
      getValue: (stat) => stat.quota,
    },
    {
      key: 'tokens',
      title: t('Total Tokens'),
      description: textOnlyTokens
        ? t('Tokens cover settled text requests only.')
        : t('Statistical tokens'),
      icon: Layers,
      iconTone: 'chart-4',
      getValue: (stat) => stat.tpm,
    },
    {
      key: 'cacheRead',
      title: t('Cache read tokens'),
      description: t('Measured cached input tokens in the selected period.'),
      icon: Layers,
      iconTone: 'info',
      getValue: (stat) => stat.cacheRead,
    },
    {
      key: 'cacheCreation',
      title: t('Cache write tokens'),
      description: t(
        'Total cache writes; lifetime details are already included.'
      ),
      icon: Layers,
      iconTone: 'chart-4',
      getValue: (stat) => stat.cacheCreation,
    },
    {
      key: 'avgRpm',
      title: t('Average RPM'),
      description: t('Requests per minute'),
      icon: Gauge,
      iconTone: 'chart-2',
      getValue: (stat, timeRangeMinutes = 1) =>
        safeDivide(stat?.rpm ?? 0, timeRangeMinutes),
    },
    {
      key: 'avgTpm',
      title: t('Average TPM'),
      description: textOnlyTokens
        ? t('Tokens cover settled text requests only.')
        : t('Tokens per minute'),
      icon: Zap,
      iconTone: 'warning',
      getValue: (stat, timeRangeMinutes = 1) =>
        stat.tpm === null ? null : safeDivide(stat.tpm, timeRangeMinutes),
    },
  ]

  if (includeOutputImages) {
    cards.push({
      key: 'images',
      title: t('Images (billing quantity)'),
      description: t('Images (billing quantity)'),
      icon: Image,
      iconTone: 'chart-4',
      getValue: (stat) => stat.outputImages,
    })
  }

  return cards
}

export function useSummaryCardsConfig(totals: {
  todayUsageDisplay: string
  usedDisplay: string
  requestCountDisplay: string
  currencyLabel: string
  todayCurrencyLabel: string
  usedCurrencyLabel: string
  currencyEnabled: boolean
}) {
  const { t } = useTranslation()

  return [
    {
      key: 'todayUsage',
      title: t('Last 24h usage'),
      value: totals.todayUsageDisplay,
      description: totals.currencyEnabled
        ? `${t('Consumed in the last 24 hours')} (${totals.todayCurrencyLabel})`
        : t('Consumed in the last 24 hours'),
      icon: Flame,
    },
    {
      key: 'usage',
      title: t('Historical Usage'),
      value: totals.usedDisplay,
      description: totals.currencyEnabled
        ? `${t('Total consumed')} (${totals.usedCurrencyLabel})`
        : t('Total consumed quota'),
      icon: TrendingUp,
    },
    {
      key: 'requests',
      title: t('Request Count'),
      value: totals.requestCountDisplay,
      description: t('Total requests made'),
      icon: Activity,
    },
  ]
}
