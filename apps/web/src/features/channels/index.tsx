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
import { Link } from '@tanstack/react-router'
import { Settings2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { useRuntimeCapabilities } from '@/context/runtime-capabilities-context'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { ROLE } from '@/lib/roles'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import { ChannelsDialogs } from './components/channels-dialogs'
import { ChannelsHealthStrip } from './components/channels-health-strip'
import { ChannelsPrimaryButtons } from './components/channels-primary-buttons'
import { ChannelsProvider } from './components/channels-provider'
import { ChannelsTable } from './components/channels-table'
import { ReadOnlyChannels } from './components/read-only-channels'
import { useChannelOps } from './use-channel-ops'

export function Channels() {
  const { t } = useTranslation()
  const isRoot = useAuthStore(
    (state) => state.auth.user?.role === ROLE.SUPER_ADMIN
  )
  const runtime = useRuntimeCapabilities()
  const localQuery = useChannelOps(!runtime)
  const channelOpsQuery = runtime?.channels ?? localQuery
  const phase = useCapabilityFreshness(channelOpsQuery)
  if (
    phase !== 'fresh' ||
    channelOpsQuery.isError ||
    channelOpsQuery.data?.data?.read_only
  ) {
    return (
      <SectionPageLayout>
        <SectionPageLayout.Title>{t('Channels')}</SectionPageLayout.Title>
        <SectionPageLayout.Content>
          {channelOpsQuery.isLoading ? (
            <LoadingState />
          ) : channelOpsQuery.isError && !channelOpsQuery.data ? (
            <ErrorState
              description={
                getServerErrorMessage(channelOpsQuery.error) ??
                t('Failed to load data')
              }
              onRetry={() => void channelOpsQuery.refetch()}
            />
          ) : (
            <>
              {phase !== 'fresh' && (
                <Alert className='mb-4' role='status'>
                  <AlertDescription>
                    <p>
                      {t(
                        'Capability information is not current. Refresh before making changes.'
                      )}
                    </p>
                    {channelOpsQuery.isError && (
                      <p>
                        {getServerErrorMessage(channelOpsQuery.error) ??
                          t('Failed to load data')}
                      </p>
                    )}
                    <Button
                      type='button'
                      variant='outline'
                      className='mt-2 min-h-11'
                      disabled={channelOpsQuery.isFetching}
                      onClick={() => void channelOpsQuery.refetch()}
                    >
                      {t('Refresh capabilities')}
                    </Button>
                  </AlertDescription>
                </Alert>
              )}
              <ReadOnlyChannels />
            </>
          )}
        </SectionPageLayout.Content>
      </SectionPageLayout>
    )
  }
  const retryTimes = channelOpsQuery.data?.data?.retry_times
  const retryLabel =
    typeof retryTimes === 'number' ? `${t('Max Retries')}: ${retryTimes}` : null
  let retryBadge = null
  if (retryLabel) {
    retryBadge = isRoot ? (
      <Tooltip>
        <TooltipTrigger
          render={
            <Badge
              variant='outline'
              className='shrink-0 cursor-pointer'
              aria-label={t('Retry Settings')}
              render={
                <Link
                  to='/system-settings/models/$section'
                  params={{ section: 'routing-reliability' }}
                />
              }
            />
          }
        >
          <span>{retryLabel}</span>
          <Settings2 data-icon='inline-end' />
        </TooltipTrigger>
        <TooltipContent>
          <p>{t('Retry Settings')}</p>
        </TooltipContent>
      </Tooltip>
    ) : (
      <Badge variant='outline' className='shrink-0'>
        {retryLabel}
      </Badge>
    )
  }

  return (
    <ChannelsProvider>
      <SectionPageLayout fixedContent>
        <SectionPageLayout.Title>
          <span className='flex min-w-0 items-center gap-2'>
            <span className='truncate'>{t('Channels')}</span>
            {retryBadge}
          </span>
        </SectionPageLayout.Title>
        <SectionPageLayout.Actions>
          <ChannelsPrimaryButtons />
        </SectionPageLayout.Actions>
        <SectionPageLayout.Content>
          <div className='flex h-full min-h-0 flex-col gap-2.5 sm:gap-3'>
            <ChannelsHealthStrip />
            <ChannelsTable />
          </div>
        </SectionPageLayout.Content>
      </SectionPageLayout>

      <ChannelsDialogs />
    </ChannelsProvider>
  )
}
