/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'

const pluginSchema = z.object({
  name: z.string(),
  version: z.string(),
  api_version: z.number().int(),
  available: z.boolean(),
  installed: z.boolean(),
  enabled: z.boolean(),
  registration_sha256: z.string(),
})

const pluginListSchema = z.object({
  success: z.literal(true),
  data: z.array(pluginSchema),
})

export type PluginInfo = z.infer<typeof pluginSchema>

export async function getPlugins(scope: NativeUsersScope, signal: AbortSignal) {
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  const response = await api.get<unknown>(
    '/api/extensions/plugins',
    nativeUsersOptions(scope, signal)
  )
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  return pluginListSchema.parse(response.data).data
}
