/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { ChartNoAxesCombined } from 'lucide-react'
import { lazy, Suspense, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { getUserQuotaDates } from '@/features/dashboard/api'
import { toIntlLocale } from '@/i18n/languages'
import { formatHistoricalRawQuota } from '@/lib/historical-quota-money'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import {
  buildOverviewUsage,
  overviewUsageRange,
  type OverviewMetric,
} from './overview-personalization-data'

const ChartCard = lazy(() =>
  import('@/features/assistant/assistant-visualization').then((module) => ({
    default: module.AssistantVisualizationCard,
  }))
)

export function OverviewUsageCharts() {
  const { t, i18n } = useTranslation()
  const user = useAuthStore((state) => state.auth.user)
  const isAdmin = Boolean(user && user.role >= ROLE.ADMIN)
  const [days, setDays] = useState(7)
  const [metric, setMetric] = useState<OverviewMetric>('requests')
  const query = useQuery({
    queryKey: ['overview-recorded-usage', user?.id, isAdmin, days],
    queryFn: async ({ signal }) => {
      const range = overviewUsageRange(days)
      const data = await getUserQuotaDates(range, isAdmin, signal)
      return {
        rows: data.data,
        coverage: data.coverage,
        availability: data.availability,
        ...range,
      }
    },
    enabled: Boolean(user?.id),
    staleTime: 60_000,
    refetchInterval: 60_000,
    retry: false,
  })
  const partialAdminUsage =
    isAdmin &&
    (query.data?.availability === 'partial' ||
      query.data?.coverage?.complete !== true ||
      query.data?.coverage?.historical_imports !== true)
  const provenance = isAdmin
    ? t('All accounts: native Core usage')
    : t('Recorded account usage')
  const usage = useMemo(() => {
    if (!query.data) return undefined
    try {
      return buildOverviewUsage(
        query.data.rows,
        metric,
        query.data.start_timestamp,
        query.data.end_timestamp,
        i18n.language,
        t('Other models'),
        t('Unknown model'),
        { includeUnreportedDays: !partialAdminUsage }
      )
    } catch {
      return undefined
    }
  }, [query.data, metric, i18n.language, t, partialAdminUsage])
  const name =
    metric === 'quota'
      ? t('Raw credits')
      : metric === 'tokens'
        ? t('Tokens')
        : t('Requests')
  const formatChartValue =
    metric === 'quota'
      ? (value: number) =>
          formatHistoricalRawQuota(
            value,
            '',
            toIntlLocale(i18n.language)
          ).trimEnd()
      : undefined
  const loading = query.isPending
  const failed = query.isError || (query.isSuccess && !usage)
  const empty = query.isSuccess && !failed && query.data.rows.length === 0
  const tokensUnavailable =
    metric === 'tokens' && !empty && usage?.hasTextTokenData === false
  return (
    <section className='overview-usage-charts' aria-label={provenance}>
      <header className='overview-chart-toolbar'>
        <div>
          <p className='overview-chart-eyebrow'>
            {isAdmin ? provenance : t('Your account only')}
          </p>
          <h2 className='text-xl font-medium tracking-tight'>
            {t('Usage at a glance')}
          </h2>
        </div>
        <div className='flex flex-wrap items-center gap-2'>
          <div
            className='bg-muted/50 flex rounded-xl p-1'
            role='group'
            aria-label={t('Usage metric')}
          >
            {(['requests', 'tokens', 'quota'] as const).map((value) => (
              <Button
                type='button'
                key={value}
                size='sm'
                variant={metric === value ? 'secondary' : 'ghost'}
                aria-pressed={metric === value}
                onClick={() => setMetric(value)}
              >
                {t(
                  value === 'quota'
                    ? 'Raw credits'
                    : value === 'tokens'
                      ? 'Tokens'
                      : 'Requests'
                )}
              </Button>
            ))}
          </div>
          <select
            className='border-input bg-background h-10 rounded-xl border px-3 text-sm'
            aria-label={t('Time range')}
            value={days}
            onChange={(event) => setDays(Number(event.target.value))}
          >
            <option value={7}>{t('Last 7 days')}</option>
            <option value={30}>{t('Last 30 days')}</option>
          </select>
        </div>
      </header>
      {!loading && !failed && partialAdminUsage && (
        <p className='text-muted-foreground mt-3 text-sm' role='status'>
          {t(
            'Partial usage across all accounts: only native Core requests are included; imported historical usage is unavailable.'
          )}
        </p>
      )}
      {loading && (
        <div className='overview-chart-grid' aria-busy='true'>
          <Skeleton className='h-80 rounded-2xl' />
          <Skeleton className='h-80 rounded-2xl' />
        </div>
      )}
      {failed && (
        <div className='overview-chart-empty' role='alert'>
          <p>
            {t(
              'Usage could not be loaded. No zero values have been substituted.'
            )}
          </p>
          <Button
            type='button'
            variant='outline'
            onClick={() => void query.refetch()}
          >
            {t('Retry')}
          </Button>
        </div>
      )}
      {empty && partialAdminUsage && (
        <div className='overview-chart-empty' role='status'>
          {t('Unavailable')}
        </div>
      )}
      {empty && !partialAdminUsage && (
        <div className='overview-chart-empty'>
          <ChartNoAxesCombined
            className='text-muted-foreground size-8'
            aria-hidden='true'
          />
          <h3 className='font-medium'>
            {t('No recorded usage in this period')}
          </h3>
          <p className='text-muted-foreground text-sm'>
            {t('Your charts will appear after usage is recorded.')}
          </p>
        </div>
      )}
      {!loading && !failed && tokensUnavailable && (
        <div className='overview-chart-empty' role='status'>
          {t('Unavailable')}
        </div>
      )}
      {!loading && !failed && !empty && !tokensUnavailable && usage && (
        <Suspense fallback={<Skeleton className='h-80 rounded-2xl' />}>
          <div className='overview-chart-grid'>
            <ChartCard
              provenance={provenance}
              formatChartValue={formatChartValue}
              visual={{
                kind: 'chart',
                title: t('Daily usage trend'),
                chart_type: 'line',
                labels: usage.labels,
                series: [{ name, values: usage.values }],
                unit: metric === 'quota' ? t('Raw credits') : '',
                source: t('Local dates · {{days}} days', { days }),
              }}
            />
            <ChartCard
              provenance={provenance}
              formatChartValue={formatChartValue}
              visual={{
                kind: 'chart',
                title: t('Model distribution'),
                chart_type: 'donut',
                labels: usage.models.map((row) => row[0]),
                series: [{ name, values: usage.models.map((row) => row[1]) }],
                unit: metric === 'quota' ? t('Raw credits') : name,
                source: t('Top five models; the remainder is combined.'),
              }}
            />
          </div>
        </Suspense>
      )}
      {metric === 'quota' && (
        <p className='text-muted-foreground mt-3 text-xs'>
          {t('Historical monetary value unavailable')}
        </p>
      )}
      {metric === 'tokens' &&
        usage?.tokenCountScope === 'settled_text_requests' && (
          <p className='text-muted-foreground mt-3 text-xs'>
            {t('Tokens cover settled text requests only.')}
          </p>
        )}
      {typeof usage?.outputImageCount === 'number' &&
        usage.outputImageCount > 0 && (
          <p className='text-muted-foreground mt-3 text-xs'>
            {t('Images (billing quantity)')}: {usage.outputImageCount}
          </p>
        )}
      <p className='text-muted-foreground mt-3 text-xs'>
        {t(
          'Recorded usage may appear with a delay. Dates follow your browser time zone.'
        )}
      </p>
    </section>
  )
}
