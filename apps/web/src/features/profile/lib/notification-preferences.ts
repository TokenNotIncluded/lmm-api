/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

const credits = z.number().int().min(0).max(Number.MAX_SAFE_INTEGER)
const siteSchema = z.object({
  revision: z.literal(1),
  quota_warning_enabled: z.boolean(),
  quota_warning_threshold: credits.positive(),
  channel_notify_enabled: z.boolean(),
  upstream_model_update_notify_enabled: z.boolean(),
})
const viewSchema = z.object({
  user_id: z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
  notify_type: z.string(),
  quota_warning_enabled: z.boolean(),
  channel_notify_enabled: z.boolean(),
  upstream_model_update_notify_enabled: z.boolean(),
  quota_warning_threshold: credits,
  notification_email: z.string().max(254),
  verified_email_available: z.boolean(),
  admin_notifications_allowed: z.boolean(),
  site: siteSchema.nullable(),
})
export type NotificationPreferences = z.infer<typeof viewSchema>
export type NotificationDraft = Pick<
  NotificationPreferences,
  | 'quota_warning_enabled'
  | 'quota_warning_threshold'
  | 'channel_notify_enabled'
  | 'upstream_model_update_notify_enabled'
>
export type NotificationScope = { userId: number; sessionId: string | undefined }

export function notificationScope(): NotificationScope | undefined {
  const { user, session } = useAuthStore.getState().auth
  return user && Number.isSafeInteger(user.id) && user.id > 0
    ? { userId: user.id, sessionId: session?.sid }
    : undefined
}
export function sameNotificationScope(scope: NotificationScope): boolean {
  const current = notificationScope()
  return (
    current?.userId === scope.userId && current?.sessionId === scope.sessionId
  )
}
export function notificationPatch(
  view: NotificationPreferences,
  draft: NotificationDraft
) {
  if (
    !credits.safeParse(draft.quota_warning_threshold).success ||
    typeof draft.quota_warning_enabled !== 'boolean' ||
    typeof draft.channel_notify_enabled !== 'boolean' ||
    typeof draft.upstream_model_update_notify_enabled !== 'boolean'
  ) {
    throw new Error('Invalid notification preferences')
  }
  // Selecting email is explicit on Save. Destination always follows the
  // server's currently verified address; other profile/permission keys never
  // enter this endpoint and remain untouched in storage.
  return {
    notify_type: 'email' as const,
    notification_email: '',
    quota_warning_enabled: draft.quota_warning_enabled,
    quota_warning_threshold: draft.quota_warning_threshold,
    ...(view.admin_notifications_allowed
      ? {
          channel_notify_enabled: draft.channel_notify_enabled,
          upstream_model_update_notify_enabled:
            draft.upstream_model_update_notify_enabled,
        }
      : {}),
  }
}
function read(
  value: unknown,
  scope: NotificationScope
): NotificationPreferences {
  const response = z
    .object({ success: z.literal(true), data: viewSchema })
    .parse(value)
  if (!sameNotificationScope(scope) || response.data.user_id !== scope.userId) {
    throw new Error('Notification session changed')
  }
  return response.data
}
export async function loadNotificationPreferences(
  scope: NotificationScope,
  signal?: AbortSignal
) {
  if (!sameNotificationScope(scope))
    throw new Error('Notification session changed')
  const response = await api.get('/api/user/notifications', {
    authScope: scope,
    signal,
    skipBusinessError: true,
    skipErrorHandler: true,
    skipAuthRefresh: true,
  })
  return read(response.data, scope)
}
export async function saveNotificationPreferences(
  scope: NotificationScope,
  view: NotificationPreferences,
  draft: NotificationDraft,
  signal?: AbortSignal
) {
  if (!sameNotificationScope(scope) || view.user_id !== scope.userId)
    throw new Error('Notification session changed')
  const response = await api.put(
    '/api/user/notifications',
    notificationPatch(view, draft),
    {
      authScope: scope,
      signal,
      skipBusinessError: true,
      skipErrorHandler: true,
      skipAuthRefresh: true,
    }
  )
  return read(response.data, scope)
}
