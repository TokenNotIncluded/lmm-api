/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useQueries } from '@tanstack/react-query'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { nativeRuntimeSchema } from '../catalog-contract'
import type { ModelRuntimeState } from '../types'

export function useModelRuntime(names: string[], enabled = true) {
  const { user, session } = useAuthStore((state) => state.auth)
  const sorted = [...new Set(names)].sort()
  const batches: string[][] = []
  for (let i = 0; i < sorted.length; i += 200) {
    batches.push(sorted.slice(i, i + 200))
  }
  return useQueries({
    queries: batches.map((models) => ({
      queryKey: [
        'model-runtime',
        user?.id ?? 0,
        session?.sid ?? null,
        user?.group ?? '',
        user?.developer_access_granted,
        models,
      ],
      queryFn: async ({ signal }) => {
        const response = await api.post<unknown>(
          '/core/v1/pricing/runtime',
          { models },
          {
            signal,
            timeout: 10_000,
            skipErrorHandler: true,
            skipBusinessError: true,
            skipAuthRefresh: !user,
            authScope: { userId: user?.id, sessionId: session?.sid },
            authMode: user ? 'session' : 'anonymous',
          }
        )
        const document = nativeRuntimeSchema.parse(response.data)
        if (document.visibility !== (user ? 'session' : 'public')) {
          throw new Error('Model runtime visibility did not match the request')
        }
        return document.data
      },
      enabled,
      staleTime: 15_000,
      refetchInterval: 30_000,
      retry: false,
    })),
    combine: (queries) => {
      const result: Record<string, ModelRuntimeState> = {}
      for (const query of queries) {
        if (!query.isError && query.data) Object.assign(result, query.data)
      }
      return result
    },
  })
}
