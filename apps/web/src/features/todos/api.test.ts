/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { afterEach, describe, test } from 'node:test'

import { api } from '@/lib/api'

import { getTodos, markAllTodosRead, markTodoRead, type TodoItem } from './api'

const originalGet = api.get
const originalPost = api.post

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
})

describe('unified to-do API', () => {
  test('loads the submitted challenge review category', async () => {
    let requestedURL = ''
    api.get = (async (url) => {
      requestedURL = url
      return {
        data: {
          success: true,
          data: {
            items: [],
            page: 1,
            page_size: 50,
            total: 0,
            category: 'open_source_bounty_review',
            unread_count: 0,
            total_unread_count: 0,
            unread_by_category: {},
            categories: [],
          },
        },
      }
    }) as typeof api.get

    await getTodos('open_source_bounty_review')
    assert.equal(
      requestedURL,
      '/api/todos?category=open_source_bounty_review&p=1&page_size=50'
    )
  })

  test('marks only the visible source item or all categories explicitly', async () => {
    const posts: Array<{ url: string; body: unknown }> = []
    api.post = (async (url, body) => {
      posts.push({ url, body })
      return { data: { success: true, data: { marked: 1 } } }
    }) as typeof api.post
    const item = {
      id: 'open_source_bounty_review:12',
      source_id: 12,
      category: 'open_source_bounty_review',
      type: 'challenge_submitted',
      title: 'open_source_bounty.challenge_submitted',
      summary: 'Review this fix',
      read: false,
      created_at: 1,
      updated_at: 1,
    } satisfies TodoItem

    await markTodoRead(item)
    await markAllTodosRead()

    assert.deepEqual(posts, [
      {
        url: '/api/todos/read',
        body: {
          category: 'open_source_bounty_review',
          ids: [12],
          all: false,
        },
      },
      {
        url: '/api/todos/read',
        body: { category: 'all', ids: [], all: true },
      },
    ])
  })
})
