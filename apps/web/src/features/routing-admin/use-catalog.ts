/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useCallback, useEffect, useMemo, useState } from 'react'

import { useAuthStore } from '@/stores/auth-store'

import { getCatalog, type Scope } from './api'
import type { Catalog } from './contract'
export function useCatalog() {
  const userId = useAuthStore((s) => s.auth.user?.id),
    sessionId = useAuthStore((s) => s.auth.session?.sid),
    role = useAuthStore((s) => s.auth.user?.role)
  const scope = useMemo<Scope | undefined>(
    () =>
      userId && role !== undefined ? { userId, sessionId, role } : undefined,
    [userId, sessionId, role]
  )
  const [catalog, setCatalog] = useState<Catalog>(),
    [error, setError] = useState<unknown>(),
    [loading, setLoading] = useState(true)
  const reload = useCallback(
    async (signal?: AbortSignal) => {
      setLoading(true)
      setError(undefined)
      try {
        if (!scope) throw Error('No session')
        const value = await getCatalog(scope, signal)
        if (!signal?.aborted) setCatalog(value)
      } catch (error) {
        if (!signal?.aborted) setError(error)
      } finally {
        if (!signal?.aborted) setLoading(false)
      }
    },
    [scope]
  )
  useEffect(() => {
    setCatalog(undefined)
    const controller = new AbortController()
    void reload(controller.signal)
    return () => controller.abort()
  }, [reload])
  return { scope, catalog, error, loading, reload }
}
