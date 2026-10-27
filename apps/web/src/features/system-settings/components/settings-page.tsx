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
/*
Copyright (C) 2026 LIghtJUNction
*/
import { useParams } from '@tanstack/react-router'
import { Suspense, useMemo, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

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
import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout'
import { PageFooterPortal } from '@/components/layout/components/page-footer'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'

import './settings-workspace.css'
import { Button } from '@/components/ui/button'

import { useSystemOptions, getOptionValue } from '../hooks/use-system-options'
import {
  settingsUseRestrictedView,
  writableOptionKey,
} from '../hooks/option-writability'
import type {
  SystemOption,
  SystemOptionsCapabilities,
} from '../types'
import { lazyNamedSection } from '../utils/lazy-section'
import { getSettingsErrorMessage } from '../utils/settings-error-message'
import { SettingsPageProvider } from './settings-page-context'
import { SettingsBreadcrumb, SettingsSearch } from './settings-search'

type SettingsPageProps<
  TSettings extends Record<string, string | number | boolean | unknown[]>,
  TSectionId extends string,
  TExtraArgs extends unknown[] = [],
> = {
  routePath: string
  defaultSettings: TSettings
  defaultSection: TSectionId
  getSectionContent: (
    sectionId: TSectionId,
    settings: TSettings,
    ...extraArgs: TExtraArgs
  ) => ReactNode
  getSectionMeta: (sectionId: TSectionId) => {
    titleKey: string
  }
  extraArgs?: TExtraArgs
  loadingMessage?: string
  resolveSettings?: (
    settings: TSettings,
    raw: SystemOption[] | undefined
  ) => TSettings
}

type SettingsPageFrameProps = {
  title: ReactNode
  children: ReactNode
}

export function SettingsPageFrame(props: SettingsPageFrameProps) {
  const [formActionsContainer, setFormActionsContainer] =
    useState<HTMLDivElement | null>(null)
  const [actionsContainer, setActionsContainer] =
    useState<HTMLDivElement | null>(null)
  const [titleStatusContainer, setTitleStatusContainer] =
    useState<HTMLSpanElement | null>(null)

  return (
    <SettingsPageProvider
      actionsContainer={actionsContainer}
      formActionsContainer={formActionsContainer}
      titleStatusContainer={titleStatusContainer}
    >
      <SectionPageLayout className='settings-workspace'>
        <SectionPageLayout.Breadcrumb>
          <SettingsBreadcrumb />
        </SectionPageLayout.Breadcrumb>
        <SectionPageLayout.Title>
          <span className='inline-flex max-w-full min-w-0 flex-wrap items-center gap-3 align-middle'>
            <span className='truncate'>{props.title}</span>
            <span
              ref={setTitleStatusContainer}
              className='inline-flex min-w-0 shrink-0 items-center'
            />
          </span>
        </SectionPageLayout.Title>
        <SectionPageLayout.Actions>
          <SettingsSearch />
          <div
            ref={setActionsContainer}
            className='flex flex-wrap items-center justify-end gap-2'
          />
        </SectionPageLayout.Actions>
        <SectionPageLayout.Content>
          <div className='settings-sheet'>{props.children}</div>
          <PageFooterPortal>
            <div className='settings-save-dock'>
              <span className='settings-save-context text-muted-foreground text-sm'>
                {props.title}
              </span>
              <div
                ref={setFormActionsContainer}
                className='settings-save-target flex flex-wrap items-center justify-end gap-2'
              />
            </div>
          </PageFooterPortal>
        </SectionPageLayout.Content>
      </SectionPageLayout>
    </SettingsPageProvider>
  )
}

/** Show only returned values, without mounting editors or filling missing defaults. */
const LimitedNoticeSection = lazyNamedSection(
  () => import('../maintenance/notice-section'),
  'NoticeSection'
)
const LimitedAnnouncementsSection = lazyNamedSection(
  () => import('../content/announcements-section'),
  'AnnouncementsSection'
)

function LimitedSettings({
  options,
  readOnly,
  partial,
  activeSection,
  capabilities,
}: {
  options: SystemOption[]
  readOnly: boolean
  partial: boolean
  activeSection: string
  capabilities?: SystemOptionsCapabilities
}) {
  // @context-fold begin limited-settings-view
  const { t } = useTranslation()
  const writableNotice =
    !readOnly &&
    activeSection === 'notice' &&
    options.some((option) => option.key === 'Notice') &&
    writableOptionKey(capabilities, 'Notice') === 'Notice'
  const announcementOption = options.find(
    (option) =>
      option.key === 'console_setting.announcements' ||
      option.key === 'Announcements'
  )
  const writableAnnouncements =
    !readOnly &&
    activeSection === 'announcements' &&
    announcementOption !== undefined &&
    writableOptionKey(capabilities, announcementOption.key) !== undefined
  const visibleOptions = options.filter((option) => {
    if (writableNotice && option.key === 'Notice') return false
    if (
      writableAnnouncements &&
      (option.key === 'Announcements' ||
        option.key === 'console_setting.announcements')
    ) {
      return false
    }
    return true
  })
  return (
    <div className='space-y-4'>
      <Alert>
        <AlertTitle>{readOnly ? t('Read only') : t('Settings')}</AlertTitle>
        <AlertDescription className='space-y-2'>
          {partial ? (
            <p>
              {t(
                'Only part of the settings is available. Missing settings are not displayed.'
              )}
            </p>
          ) : null}
          {readOnly ? (
            <p>
              {t(
                'Settings are read-only. Apply changes through the host configuration upgrade procedure.'
              )}
            </p>
          ) : null}
        </AlertDescription>
      </Alert>
      {visibleOptions.length > 0 ? (
        <dl className='divide-y border'>
          {visibleOptions.map((option) => (
            <div
              key={option.key}
              className='grid min-w-0 gap-2 p-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]'
            >
              <dt className='text-sm font-medium break-words'>
                {option.key}
                <span className='text-muted-foreground ml-2 text-xs'>
                  {t('Read only')}
                </span>
              </dt>
              <dd className='min-w-0 font-mono text-sm break-words whitespace-pre-wrap'>
                {option.value}
              </dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className='text-muted-foreground text-sm'>
          {t('No data available')}
        </p>
      )}
      {writableNotice ? (
        <LimitedNoticeSection
          defaultValue={
            options.find((option) => option.key === 'Notice')?.value ?? ''
          }
        />
      ) : null}
      {writableAnnouncements && announcementOption ? (
        <LimitedAnnouncementsSection
          enabled={(() => {
            const enabledOption = options.find(
              (option) =>
                option.key === 'console_setting.announcements_enabled'
            )
            return enabledOption
              ? enabledOption.value === 'true'
              : undefined
          })()}
          enabledWritable={false}
          data={announcementOption.value}
        />
      ) : null}
      <div className='flex justify-end'>
        <Button type='button' disabled>
          {t('Save Settings')}
        </Button>
      </div>
    </div>
  )
  // @context-fold end limited-settings-view
}

/**
 * Generic settings page component
 * Handles loading state, data fetching, and section rendering
 */
export function SettingsPage<
  TSettings extends Record<string, string | number | boolean | unknown[]>,
  TSectionId extends string,
  TExtraArgs extends unknown[] = [],
>({
  routePath,
  defaultSettings,
  defaultSection,
  getSectionContent,
  getSectionMeta,
  extraArgs,
  loadingMessage = 'Loading settings...',
  resolveSettings,
}: SettingsPageProps<TSettings, TSectionId, TExtraArgs>) {
  const { t } = useTranslation()
  const { data, isLoading, isError, error, isFetching, refetch } =
    useSystemOptions()
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const params = useParams({ from: routePath as any })
  const activeSection = (params?.section ?? defaultSection) as TSectionId
  const sectionMeta = getSectionMeta(activeSection)
  const readOnly = data?.capabilities?.read_only === true
  const partial = data?.capabilities?.partial === true
  const restricted =
    settingsUseRestrictedView(data?.capabilities) || readOnly || partial

  const settings = useMemo(() => {
    if (restricted) return null
    const baseSettings = getOptionValue(
      data?.data,
      defaultSettings
    ) as TSettings
    return resolveSettings
      ? resolveSettings(baseSettings, data?.data)
      : baseSettings
  }, [data?.data, defaultSettings, resolveSettings, restricted])

  if (isLoading && !data) {
    return (
      <SettingsPageFrame title={t(sectionMeta.titleKey)}>
        <LoadingState message={t(loadingMessage)} />
      </SettingsPageFrame>
    )
  }

  if (isError && !data) {
    return (
      <SettingsPageFrame title={t(sectionMeta.titleKey)}>
        <ErrorState
          title={t('Unable to load settings')}
          description={getSettingsErrorMessage(
            error,
            t('Failed to load settings')
          )}
          onRetry={() => {
            void refetch()
          }}
        />
      </SettingsPageFrame>
    )
  }

  const sectionContent =
    settings === null ? (
      <LimitedSettings
        options={data?.data ?? []}
        readOnly={readOnly}
        partial={partial}
        activeSection={activeSection}
        capabilities={data?.capabilities}
      />
    ) : (
      getSectionContent(
        activeSection,
        settings,
        ...((extraArgs ?? []) as TExtraArgs)
      )
    )

  return (
    <SettingsPageFrame title={t(sectionMeta.titleKey)}>
      {isError ? (
        <Alert className='mb-4'>
          <AlertTitle>{t('Unable to refresh settings')}</AlertTitle>
          <AlertDescription>
            <p>
              {t(
                'The last loaded settings and your current edits are still available.'
              )}
            </p>
            <p>
              {getSettingsErrorMessage(error, t('Failed to load settings'))}
            </p>
            <Button
              type='button'
              variant='outline'
              size='sm'
              className='mt-2 min-h-11 sm:min-h-9'
              disabled={isFetching}
              onClick={() => {
                void refetch()
              }}
            >
              {t('Retry')}
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}
      <Suspense fallback={<LoadingState message={t(loadingMessage)} />}>
        {sectionContent}
      </Suspense>
    </SettingsPageFrame>
  )
}
