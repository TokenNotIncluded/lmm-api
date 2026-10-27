/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'

import {
  readNativeUsersScope,
  nativeUsersOptions,
} from '@/features/native-users/access'
import { CAPABILITY_REFRESH_MS } from '@/lib/runtime-capabilities'
import { useAuthStore } from '@/stores/auth-store'

import { getChannelOps } from './api'

export function useChannelOps(enabled = true) {
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  return useQuery({
    queryKey: ['channel-ops'],
    queryFn: ({ signal }) =>
      getChannelOps(scope ? nativeUsersOptions(scope, signal) : { signal }),
    enabled,
    retry: false,
    staleTime: CAPABILITY_REFRESH_MS,
    refetchInterval: CAPABILITY_REFRESH_MS,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: 'always',
    refetchOnReconnect: 'always',
  })
}
