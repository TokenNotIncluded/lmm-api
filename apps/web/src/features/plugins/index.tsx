/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { ZodError } from 'zod'

import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import { getPlugins } from './api'

// @context-fold begin plugin-inventory
function PluginInventory({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: ['extension-plugins', scope.userId, scope.sessionId, scope.role],
    queryFn: ({ signal }) => getPlugins(scope, signal),
    retry: false,
  })

  if (query.isPending) return <p>{t('Loading...')}</p>
  if (query.isError) {
    return (
      <ErrorState
        description={
          (query.error instanceof ZodError
            ? null
            : getServerErrorMessage(query.error)) ??
          t('The service is temporarily unavailable. Please try again later.')
        }
        onRetry={() => void query.refetch()}
      />
    )
  }

  const enabled = query.data.filter((plugin) => plugin.enabled)
  return (
    <div className='min-w-0 space-y-5'>
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <Badge variant='outline'>{t('Read only')}</Badge>
        <Button
          variant='outline'
          className='min-h-11'
          onClick={() => void query.refetch()}
          disabled={query.isFetching}
        >
          {query.isFetching ? t('Refreshing...') : t('Refresh')}
        </Button>
      </div>
      <section className='space-y-2'>
        <h2 className='text-lg font-semibold'>{t('Enabled modules')}</h2>
        {enabled.length ? (
          <div className='flex flex-wrap gap-2'>
            {enabled.map((plugin) => (
              <Badge
                key={plugin.name}
                variant='secondary'
                className='max-w-full [overflow-wrap:anywhere] whitespace-normal'
              >
                {plugin.name}
              </Badge>
            ))}
          </div>
        ) : (
          <p className='text-muted-foreground text-sm'>
            {t('No data available')}
          </p>
        )}
      </section>
      <section className='space-y-3'>
        <h2 className='text-lg font-semibold'>{t('Plugin management')}</h2>
        {query.data.length === 0 && (
          <p className='text-muted-foreground text-sm'>
            {t('No data available')}
          </p>
        )}
        {query.data.map((plugin) => (
          <article
            key={plugin.name}
            className='min-w-0 space-y-3 rounded-lg border p-4'
          >
            <h3 className='font-semibold [overflow-wrap:anywhere]'>
              {plugin.name}
            </h3>
            <dl className='grid grid-cols-1 gap-3 text-sm sm:grid-cols-2 lg:grid-cols-3'>
              <div>
                <dt className='text-muted-foreground'>{t('Version')}</dt>
                <dd className='[overflow-wrap:anywhere]'>{plugin.version}</dd>
              </div>
              <div>
                <dt className='text-muted-foreground'>{t('API version')}</dt>
                <dd>{plugin.api_version}</dd>
              </div>
              <div>
                <dt className='text-muted-foreground'>
                  {t('Module availability')}
                </dt>
                <dd>{plugin.available ? t('Available') : t('Unavailable')}</dd>
              </div>
              <div>
                <dt className='text-muted-foreground'>
                  {t('Installation status')}
                </dt>
                <dd>
                  {plugin.installed ? t('Installed') : t('Not installed')}
                </dd>
              </div>
              <div>
                <dt className='text-muted-foreground'>{t('Status')}</dt>
                <dd>{plugin.enabled ? t('Enabled') : t('Disabled')}</dd>
              </div>
            </dl>
          </article>
        ))}
      </section>
      <p className='text-muted-foreground text-sm'>
        {t('Capability details are not reported by this API.')}
      </p>
    </div>
  )
}
// @context-fold end plugin-inventory

export function PluginsPage() {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>
        {t('Plugin management')}
      </SectionPageLayout.Title>
      <SectionPageLayout.Content>
        {scope ? (
          <PluginInventory
            key={`${scope.userId}:${scope.sessionId}:${scope.role}`}
            scope={scope}
          />
        ) : (
          <Alert role='alert'>
            <AlertDescription>
              {t('A current administrator session is required.')}
            </AlertDescription>
          </Alert>
        )}
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
