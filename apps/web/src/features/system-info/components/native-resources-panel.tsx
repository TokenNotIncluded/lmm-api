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
*/
import { useQuery } from '@tanstack/react-query'
import { RefreshCw, ServerCog } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'

import { getNativeSystemResources } from '../api'
import type { SystemProcessInfo } from '../types'

const POLL_INTERVAL_MS = 30_000

function percent(value?: number) {
  return typeof value === 'number'
    ? `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(value)}%`
    : '-'
}

function bytes(value?: number) {
  if (typeof value !== 'number') return '-'
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(value / 1024 / 1024)} MB`
}

function Metric({ label, value }: { label: string; value: string | number }) {
  return (
    <div className='flex items-center justify-between gap-3 text-xs'>
      <span className='text-muted-foreground'>{label}</span>
      <span className='font-mono tabular-nums'>{value}</span>
    </div>
  )
}

function ProcessCard({
  label,
  process,
}: {
  label: string
  process?: SystemProcessInfo
}) {
  const { t } = useTranslation()
  return (
    <div className='bg-card space-y-2 border p-3'>
      <div className='text-sm font-medium'>{label}</div>
      {process ? (
        <>
          <Metric label={t('PID')} value={process.pid ?? t('Not reported')} />
          <Metric label={t('CPU')} value={percent(process.cpu_percent)} />
          <Metric label={t('RSS')} value={bytes(process.memory_bytes)} />
        </>
      ) : (
        <div className='text-muted-foreground text-xs'>{t('Not reported')}</div>
      )}
    </div>
  )
}

export function NativeResourcesPanel() {
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: ['system-info', 'native-resources'],
    queryFn: getNativeSystemResources,
    staleTime: POLL_INTERVAL_MS,
    refetchInterval: POLL_INTERVAL_MS,
    retry: false,
  })

  return (
    <section className='bg-card overflow-hidden border'>
      <div className='flex items-center justify-between gap-3 border-b px-4 py-3'>
        <div className='flex min-w-0 items-center gap-2'>
          <ServerCog
            className='text-muted-foreground size-4 shrink-0'
            aria-hidden='true'
          />
          <h3 className='text-sm font-semibold'>
            {t('Service resource breakdown')}
          </h3>
        </div>
        <Button
          type='button'
          variant='outline'
          size='sm'
          onClick={() => void query.refetch()}
          disabled={query.isFetching}
          aria-label={t('Refresh')}
        >
          <RefreshCw className='size-3.5' aria-hidden='true' />
          {query.isFetching ? t('Refreshing...') : t('Refresh')}
        </Button>
      </div>
      <div className='p-4' aria-busy={query.isFetching}>
        {query.isLoading ? (
          <div className='grid gap-3 sm:grid-cols-3'>
            {[0, 1, 2].map((key) => (
              <Skeleton key={key} className='h-28' />
            ))}
          </div>
        ) : query.isError ? (
          <ErrorState
            onRetry={() => void query.refetch()}
            className='min-h-[220px]'
          />
        ) : query.data ? (
          <div className='space-y-3'>
            <p className='text-muted-foreground text-xs'>
              {t(
                'System samples describe the host. Core and Extensions values describe their own processes.'
              )}
            </p>
            <div className='grid gap-3 sm:grid-cols-3'>
              <div className='bg-card space-y-2 border p-3'>
                <div className='text-sm font-medium'>{t('System')}</div>
                <Metric
                  label={t('CPU')}
                  value={percent(query.data.system.cpu_usage_percent)}
                />
                <Metric
                  label={t('Memory')}
                  value={percent(query.data.system.memory_usage_percent)}
                />
                <Metric
                  label={t('Used')}
                  value={bytes(query.data.system.memory_used_bytes)}
                />
              </div>
              <ProcessCard
                label={t('Core process')}
                process={query.data.processes.core}
              />
              <ProcessCard
                label={t('Extensions process')}
                process={query.data.processes.extensions}
              />
            </div>
          </div>
        ) : null}
      </div>
    </section>
  )
}
