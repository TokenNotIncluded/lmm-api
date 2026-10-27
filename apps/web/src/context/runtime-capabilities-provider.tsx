/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, type ReactNode } from 'react'

import { useChannelOps } from '@/features/channels/use-channel-ops'
import { readNativeUsersScope } from '@/features/native-users/access'
import { usePlugins } from '@/features/plugins/use-plugins'
import { useSystemOptions } from '@/features/system-settings/hooks/use-system-options'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { useStatus } from '@/hooks/use-status'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { RuntimeCapabilitiesContext } from './runtime-capabilities-context'

function useSnapshot() {
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  const client = useQueryClient()
  const core = useStatus()
  const plugins = usePlugins(scope)
  const channels = useChannelOps(Boolean(scope))
  const settings = useSystemOptions(scope?.role === ROLE.SUPER_ADMIN)
  const pluginPhase = useCapabilityFreshness(plugins)
  const channelPhase = useCapabilityFreshness(channels)
  const settingsPhase = useCapabilityFreshness(settings)
  const previous = useRef<string | null>(null)
  const registry = plugins.data ? JSON.stringify(plugins.data) : null
  const scopeKey = scope
    ? `${scope.userId}:${scope.sessionId}:${scope.role}`
    : null

  useEffect(() => {
    previous.current = null
  }, [scopeKey])
  useEffect(() => {
    if (pluginPhase !== 'fresh' || registry === null) return
    const changed = previous.current !== null && previous.current !== registry
    previous.current = registry
    if (changed) {
      // No reload operation is exposed by the current read-only UI. A changed
      // registry (including an external administrator's change) invalidates
      // existing capability/config reads without replaying any write.
      for (const queryKey of [
        ['status'],
        ['system-options'],
        ['channel-ops'],
      ]) {
        void client.invalidateQueries({ queryKey })
      }
    }
  }, [client, pluginPhase, registry])

  return {
    core,
    plugins,
    channels,
    settings,
    pluginPhase,
    channelPhase,
    settingsPhase,
    hasPluginInventoryAccess: Boolean(scope),
    // This DTO has no allowed_actions, atomic generation or write receipt.
    pluginLifecycleWritable: false as const,
  }
}

export type RuntimeCapabilitiesSnapshot = ReturnType<typeof useSnapshot>

export function RuntimeCapabilitiesProvider({
  children,
}: {
  children: ReactNode
}) {
  const snapshot = useSnapshot()
  return (
    <RuntimeCapabilitiesContext value={snapshot}>
      {children}
    </RuntimeCapabilitiesContext>
  )
}
