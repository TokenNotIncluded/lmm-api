/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { api } from '@/lib/api'

import { getFinanceOverview, getFinanceUser } from './api'

describe('finance payment-method filters', () => {
  test('keeps a selected payment method through overview and user drill-down requests', async () => {
    const originalGet = api.get
    const requests: Array<{ url: string; config?: unknown }> = []
    api.get = (async (url: string, config?: unknown) => {
      requests.push({ url, config })
      return { data: { success: true, data: {} } }
    }) as typeof api.get

    try {
      await getFinanceOverview(30, 'waffo_pancake')
      await getFinanceUser(42, 30, 'waffo_pancake')

      assert.equal(requests[0]?.url, '/api/finance/overview')
      assert.equal(requests[1]?.url, '/api/finance/users/42')
      for (const request of requests) {
        const params = (request.config as { params?: Record<string, unknown> })
          .params
        assert.equal(params?.payment_method, 'waffo_pancake')
      }
    } finally {
      api.get = originalGet
    }
  })
})
