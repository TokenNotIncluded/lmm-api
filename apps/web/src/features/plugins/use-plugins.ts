/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'

import type { NativeUsersScope } from '@/features/native-users/access'
import { CAPABILITY_REFRESH_MS } from '@/lib/runtime-capabilities'

import { getPlugins } from './api'

export function usePlugins(scope: NativeUsersScope | undefined) {
  return useQuery({
    queryKey: [
      'extension-plugins',
      scope?.userId,
      scope?.sessionId,
      scope?.role,
    ],
    queryFn: ({ signal }) => {
      if (!scope) {
        throw new Error('A current administrator session is required.')
      }
      return getPlugins(scope, signal)
    },
    enabled: Boolean(scope),
    retry: false,
    staleTime: CAPABILITY_REFRESH_MS,
    refetchInterval: CAPABILITY_REFRESH_MS,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: 'always',
    refetchOnReconnect: 'always',
  })
}
