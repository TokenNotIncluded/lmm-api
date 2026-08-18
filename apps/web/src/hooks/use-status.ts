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
import { useQuery } from '@tanstack/react-query'

import type { SystemStatus } from '@/features/auth/types'
import { getStatus } from '@/lib/api'
import {
  getCapabilitySafeStatus,
  normalizeBackendCapabilities,
} from '@/lib/backend-capabilities'
import { useAuthStore } from '@/stores/auth-store'
import { useSystemConfigStore } from '@/stores/system-config-store'

import { mapStatusDataToConfig } from './use-system-config'

// Get initial cache from localStorage
function getInitialStatus(): SystemStatus | undefined {
  try {
    if (typeof window !== 'undefined') {
      const saved = window.localStorage.getItem('status')
      return saved
        ? normalizeBackendCapabilities(JSON.parse(saved) as SystemStatus, false)
        : undefined
    }
  } catch {
    /* empty */
  }
  return undefined
}

export function useStatus() {
  const statusScope = useAuthStore((state) => {
    const user = state.auth.user
    if (!user) return 'anonymous'
    return `user:${user.id}:docs:${user.permissions?.docs_access === true ? 1 : 0}`
  })
  const { data, isFetchedAfterMount, isFetching, isLoading, error, refetch } =
    useQuery({
      queryKey: ['status', statusScope],
      queryFn: async () => {
        const rawStatus = await getStatus()
        const status = rawStatus
          ? normalizeBackendCapabilities(rawStatus as SystemStatus)
          : null
        try {
          if (status) {
            const { setConfig } = useSystemConfigStore.getState()
            setConfig(
              mapStatusDataToConfig(
                status as Parameters<typeof mapStatusDataToConfig>[0]
              )
            )
          }
        } catch (err) {
          if (import.meta.env.DEV) {
            // eslint-disable-next-line no-console
            console.warn(
              '[useStatus] Failed to sync status to system config',
              err
            )
          }
        }
        // Save to localStorage
        try {
          if (typeof window !== 'undefined' && status) {
            window.localStorage.setItem('status', JSON.stringify(status))
          }
        } catch {
          /* empty */
        }
        return status as SystemStatus | null
      },
      // Use localStorage data as initial data
      placeholderData: getInitialStatus(),
      // Capability decisions require a live response for this observer mount.
      staleTime: 0,
      refetchOnMount: 'always',
      refetchOnWindowFocus: 'always',
      refetchOnReconnect: 'always',
      refetchInterval: 30_000,
      refetchIntervalInBackground: true,
      retry: 1,
      retryDelay: 1_000,
      // Cache expires after 30 minutes
      gcTime: 30 * 60 * 1000,
    })

  const capabilitiesReady =
    Boolean(data) && isFetchedAfterMount && !isFetching && !error

  return {
    status: getCapabilitySafeStatus(data, capabilitiesReady),
    loading: isLoading,
    capabilitiesReady,
    error,
    refetch,
  }
}
