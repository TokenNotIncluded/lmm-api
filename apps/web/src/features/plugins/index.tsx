/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import { useTranslation } from 'react-i18next'
import { ZodError } from 'zod'

import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useRuntimeCapabilities } from '@/context/runtime-capabilities-context'
import {
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import { usePlugins } from './use-plugins'

// @context-fold begin plugin-inventory
function PluginInventory({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const runtime = useRuntimeCapabilities()
  const localQuery = usePlugins(runtime ? undefined : scope)
  const query = runtime?.plugins ?? localQuery
  const phase = useCapabilityFreshness(query)
  const status = isAxiosError(query.error)
    ? query.error.response?.status
    : undefined
  const errorMessage =
    query.error instanceof ZodError
      ? t('The plugin response does not match this release.')
      : status === 401
        ? t('Your session has expired. Sign in again.')
        : status === 403
          ? t('You do not have permission to read the plugin registry.')
          : status === 503
            ? t('The extension service is unavailable (503).')
            : (getServerErrorMessage(query.error) ??
              t(
                'The service is temporarily unavailable. Please try again later.'
              ))

  if (query.isPending) return <p>{t('Loading...')}</p>
  if (!query.data) {
    return (
      <ErrorState
        description={errorMessage}
        onRetry={() => void query.refetch()}
      />
    )
  }

  const enabled = query.data.filter((plugin) => plugin.enabled)
  return (
    <div className='min-w-0 space-y-5'>
      {phase !== 'fresh' && (
        <Alert role='status'>
          <AlertDescription>
            <p>
              {t('Plugin status is not current. Refresh before relying on it.')}
            </p>
            {query.isError && <p>{errorMessage}</p>}
          </AlertDescription>
        </Alert>
      )}
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <div className='min-w-0 space-y-2'>
          <Badge variant='outline'>{t('Read only')}</Badge>
          <p className='text-muted-foreground text-xs tabular-nums'>
            {t('Last successful read: {{time}}', {
              time: new Date(query.dataUpdatedAt).toLocaleString(),
            })}
          </p>
        </div>
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
        <div className='divide-y border-y'>
          {query.data.map((plugin) => (
            <article
              key={plugin.name}
              className='grid min-w-0 gap-3 py-4 lg:grid-cols-[minmax(12rem,1fr)_minmax(0,2fr)] lg:items-center lg:gap-6'
            >
              <div className='min-w-0'>
                <h3 className='font-semibold [overflow-wrap:anywhere]'>
                  {plugin.name}
                </h3>
                <p className='text-muted-foreground mt-1 text-xs [overflow-wrap:anywhere] tabular-nums'>
                  {t('Version')} {plugin.version} · {t('API version')}{' '}
                  {plugin.api_version}
                </p>
                <details className='mt-1 text-xs'>
                  <summary className='text-muted-foreground flex min-h-11 cursor-pointer items-center focus-visible:outline-2 focus-visible:outline-offset-2'>
                    {t('Registration fingerprint')}
                  </summary>
                  <code className='block [overflow-wrap:anywhere]'>
                    {plugin.registration_sha256}
                  </code>
                </details>
              </div>
              <dl className='grid min-w-0 grid-cols-2 gap-3 text-sm sm:grid-cols-3'>
                <div>
                  <dt className='text-muted-foreground'>
                    {t('Registered in this instance')}
                  </dt>
                  <dd>
                    {plugin.available ? t('Available') : t('Unavailable')}
                  </dd>
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
                  <dt className='text-muted-foreground'>
                    {t('Route configuration')}
                  </dt>
                  <dd>{plugin.enabled ? t('Enabled') : t('Disabled')}</dd>
                </div>
              </dl>
            </article>
          ))}
        </div>
      </section>
      <p className='text-muted-foreground text-sm'>
        {t('Registry entries are declarations, not service health checks.')}
      </p>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Lifecycle changes are unavailable: this service does not provide permitted actions or an atomic change receipt.'
        )}
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
