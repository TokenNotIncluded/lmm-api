/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { useAuthStore } from '@/stores/auth-store'

import {
  loadNotificationPreferences,
  notificationScope,
  sameNotificationScope,
  saveNotificationPreferences,
  type NotificationDraft,
  type NotificationPreferences,
} from '../lib/notification-preferences'

export function NotificationPreferencesForm({
  userId,
  onUpdate,
}: {
  userId?: number
  onUpdate: () => void
}) {
  const { t } = useTranslation()
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const activeUser = useAuthStore((state) => state.auth.user?.id)
  const key = `${userId ?? ''}:${activeUser ?? ''}:${sessionId ?? ''}`
  const request = useRef<AbortController | null>(null)
  const [reload, setReload] = useState(0)
  const [state, setState] = useState<{
    key: string
    view?: NotificationPreferences
    draft?: NotificationDraft
    busy: boolean
    message?: string
    failed?: boolean
  }>({ key, busy: true })
  const current = state.key === key ? state : { key, busy: true }
  useEffect(() => {
    const controller = new AbortController()
    request.current = controller
    setState({ key, busy: true })
    const scope = notificationScope()
    if (!scope || scope.userId !== userId) {
      setState({
        key,
        busy: false,
        failed: true,
        message: 'Unable to load notification settings.',
      })
      return () => controller.abort()
    }
    void loadNotificationPreferences(scope, controller.signal)
      .then((view) => {
        if (!controller.signal.aborted)
          setState({ key, view, draft: view, busy: false })
      })
      .catch(() => {
        if (!controller.signal.aborted)
          setState({
            key,
            busy: false,
            failed: true,
            message: 'Unable to load notification settings.',
          })
      })
      .finally(() => {
        if (request.current === controller) request.current = null
      })
    return () => {
      controller.abort()
      request.current?.abort()
    }
  }, [key, reload, userId])

  const save = async () => {
    const scope = notificationScope()
    const { view, draft } = current
    if (!scope || scope.userId !== userId || !view || !draft || current.busy)
      return
    const controller = new AbortController()
    request.current = controller
    setState({ ...current, busy: true, failed: false, message: undefined })
    try {
      const saved = await saveNotificationPreferences(
        scope,
        view,
        draft,
        controller.signal
      )
      if (!controller.signal.aborted && sameNotificationScope(scope)) {
        setState({
          key,
          view: saved,
          draft: saved,
          busy: false,
          message: 'Notification preferences saved',
        })
        onUpdate()
      }
    } catch {
      if (!controller.signal.aborted && sameNotificationScope(scope)) {
        setState({
          ...current,
          busy: false,
          failed: true,
          message: 'Failed to update settings',
        })
      }
    } finally {
      if (request.current === controller) request.current = null
    }
  }
  const change = <K extends keyof NotificationDraft>(
    name: K,
    value: NotificationDraft[K]
  ) => {
    if (!current.draft || current.busy) return
    setState({
      ...current,
      draft: { ...current.draft, [name]: value },
      message: undefined,
      failed: false,
    })
  }
  const { view, draft } = current
  return (
    <section aria-label={t('Notifications')} className='space-y-4'>
      <div>
        <h4 className='text-sm font-medium'>{t('Notifications')}</h4>
        <p className='text-muted-foreground mt-1 text-xs'>
          {t('Notifications are sent to your verified account email.')}
        </p>
      </div>
      {view && draft && (
        <>
          <div className='space-y-1.5'>
            <Label htmlFor='notification-verified-email'>
              {t('Notification Email')}
            </Label>
            <Input
              id='notification-verified-email'
              type='email'
              readOnly
              value={view.notification_email}
            />
            {!view.verified_email_available && (
              <p className='text-muted-foreground text-xs'>
                {t('Verify an email address before receiving notifications.')}
              </p>
            )}
            {view.notify_type !== 'email' && (
              <p className='text-muted-foreground text-xs'>
                {t('Save these preferences to use email notifications.')}
              </p>
            )}
          </div>
          {!view.site && (
            <p className='text-muted-foreground text-xs'>
              {t('Notifications are disabled by the site administrator.')}
            </p>
          )}
          <div className='flex items-center justify-between gap-3 rounded-none border p-3'>
            <Label htmlFor='notification-balance-enabled'>
              {t('Low balance notifications')}
            </Label>
            <Switch
              id='notification-balance-enabled'
              checked={draft.quota_warning_enabled}
              disabled={current.busy}
              onCheckedChange={(value) =>
                change('quota_warning_enabled', value)
              }
            />
          </div>
          <div className='space-y-1.5'>
            <Label htmlFor='notification-threshold'>
              {t('Threshold in credits (0 uses the site default)')}
            </Label>
            <Input
              id='notification-threshold'
              type='number'
              min={0}
              max={Number.MAX_SAFE_INTEGER}
              step={1}
              value={draft.quota_warning_threshold}
              disabled={current.busy}
              onChange={(event) =>
                change('quota_warning_threshold', Number(event.target.value))
              }
            />
            <p className='text-muted-foreground text-xs'>
              {t(
                'After a top-up raises the balance above this threshold, a later drop can trigger another reminder.'
              )}
            </p>
            {view.site && !view.site.quota_warning_enabled && (
              <p className='text-muted-foreground text-xs'>
                {t('Notifications are disabled by the site administrator.')}
              </p>
            )}
          </div>
          {view.admin_notifications_allowed && (
            <>
              <div className='flex items-center justify-between gap-3 rounded-none border p-3'>
                <Label htmlFor='notification-channel-enabled'>
                  {t('Channel notifications')}
                </Label>
                <Switch
                  id='notification-channel-enabled'
                  checked={draft.channel_notify_enabled}
                  disabled={current.busy}
                  onCheckedChange={(value) =>
                    change('channel_notify_enabled', value)
                  }
                />
              </div>
              <div className='flex items-center justify-between gap-3 rounded-none border p-3'>
                <Label htmlFor='notification-models-enabled'>
                  {t('Channel model updates')}
                </Label>
                <Switch
                  id='notification-models-enabled'
                  checked={draft.upstream_model_update_notify_enabled}
                  disabled={current.busy}
                  onCheckedChange={(value) =>
                    change('upstream_model_update_notify_enabled', value)
                  }
                />
              </div>
            </>
          )}
          <div className='flex justify-end'>
            <Button
              type='button'
              disabled={
                current.busy ||
                !Number.isSafeInteger(draft.quota_warning_threshold) ||
                draft.quota_warning_threshold < 0
              }
              onClick={save}
            >
              {current.busy
                ? t('Saving...')
                : t('Save notification preferences')}
            </Button>
          </div>
        </>
      )}
      {current.busy && !view && (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('Loading...')}
        </p>
      )}
      {current.message && (
        <p
          role={current.failed ? 'alert' : 'status'}
          className={
            current.failed
              ? 'text-destructive text-sm'
              : 'text-muted-foreground text-sm'
          }
        >
          {t(current.message)}
        </p>
      )}
      {current.failed && !view && (
        <Button
          type='button'
          variant='outline'
          onClick={() => setReload((value) => value + 1)}
        >
          {t('Retry')}
        </Button>
      )}
    </section>
  )
}
