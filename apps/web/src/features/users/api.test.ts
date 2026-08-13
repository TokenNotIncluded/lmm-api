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
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { api } from '@/lib/api'

import {
  getAssistantUserProfile,
  getUsers,
  searchUsers,
  updateAssistantUserProfile,
} from './api'

describe('user management API filters', () => {
  test('passes the L0 filter to the paginated user endpoint', async () => {
    const originalGet = api.get
    let requestConfig: unknown
    api.get = (async (url: string, config?: unknown) => {
      assert.equal(url, '/api/user/')
      requestConfig = config
      return { data: { success: true, data: { items: [], total: 0 } } }
    }) as typeof api.get

    try {
      await getUsers({ p: 2, page_size: 20, trust_level: 0 })
      assert.deepEqual(requestConfig, {
        params: {
          p: 2,
          page_size: 20,
          trust_level: 0,
          sort_by: undefined,
          sort_order: undefined,
        },
      })
    } finally {
      api.get = originalGet
    }
  })

  test('passes the L0 filter to user search', async () => {
    const originalGet = api.get
    let requestUrl = ''
    api.get = (async (url: string) => {
      requestUrl = url
      return { data: { success: true, data: { items: [], total: 0 } } }
    }) as typeof api.get

    try {
      await searchUsers({ keyword: 'alice', trust_level: 0 })
      const [path, query = ''] = requestUrl.split('?')
      assert.equal(path, '/api/user/search')
      const params = new URLSearchParams(query)
      assert.equal(params.get('keyword'), 'alice')
      assert.equal(params.get('trust_level'), '0')
    } finally {
      api.get = originalGet
    }
  })
})

describe('administrator assistant profile API', () => {
  test('uses the admin-only per-user profile endpoints', async () => {
    const originalGet = api.get
    const originalPut = api.put
    const requests: Array<{ method: string; url: string; data?: unknown }> = []
    api.get = (async (url: string) => {
      requests.push({ method: 'GET', url })
      return {
        data: {
          success: true,
          data: {
            profile_key: 'guided_buyer',
            tags: ['new-user'],
            strategy: 'Ask one question at a time.',
            enabled: true,
            updated_at: 1,
          },
        },
      }
    }) as typeof api.get
    api.put = (async (url: string, data: unknown) => {
      requests.push({ method: 'PUT', url, data })
      return {
        data: {
          success: true,
          data: {
            profile_key: 'guided_buyer',
            tags: ['new-user'],
            strategy: 'Ask one question at a time.',
            enabled: true,
            updated_at: 2,
          },
        },
      }
    }) as typeof api.put

    try {
      await getAssistantUserProfile(41)
      await updateAssistantUserProfile(41, {
        profile_key: 'guided_buyer',
        tags: ['new-user'],
        strategy: 'Ask one question at a time.',
        enabled: true,
      })
      assert.deepEqual(requests, [
        { method: 'GET', url: '/api/user/41/assistant-profile' },
        {
          method: 'PUT',
          url: '/api/user/41/assistant-profile',
          data: {
            profile_key: 'guided_buyer',
            tags: ['new-user'],
            strategy: 'Ask one question at a time.',
            enabled: true,
          },
        },
      ])
    } finally {
      api.get = originalGet
      api.put = originalPut
    }
  })
})
