import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  currentScope,
  getCatalog,
  pricePreview,
  publish,
  RoutingAdminError,
} from './api'
import { catalog, preview } from './fixtures.test-helper'
const get = api.get,
  post = api.post
afterEach(() => {
  api.get = get
  api.post = post
  useAuthStore.getState().auth.reset('idle')
})
function scope() {
  useAuthStore
    .getState()
    .auth.setUser({ id: 7, username: 'routing-admin', role: 100 })
  return currentScope()!
}
test('source JSON crosses network without numeric conversion', async () => {
  const current = scope(),
    source = '{"prices":{"input":9007199254740993}}'
  api.post = (async (url, body, config) => {
    assert.equal(url, '/api/core/routing-admin/prices/preview')
    assert.deepEqual(body, {
      expected: catalog.expected,
      source_json: source,
      selections: [{ model: 'model-a', adjustment: 'divide' }],
    })
    assert.deepEqual(config?.authScope, current)
    assert.equal(config?.skipAuthRefresh, true)
    return { data: { success: true, data: preview } }
  }) as typeof api.post
  assert.deepEqual(
    await pricePreview(current, catalog.expected, source, [
      { model: 'model-a', adjustment: 'divide' },
    ]),
    preview
  )
})
test('cancelled/replaced sessions reject old account results', async () => {
  const current = scope()
  api.get = (async () => {
    useAuthStore
      .getState()
      .auth.setUser({ id: 8, username: 'other', role: 100 })
    return { data: { success: true, data: catalog } }
  }) as typeof api.get
  await assert.rejects(
    getCatalog(current),
    (e) => e instanceof RoutingAdminError && e.code === 'session_changed'
  )
  const controller = new AbortController()
  controller.abort()
  await assert.rejects(
    getCatalog(current, controller.signal),
    (e) => e instanceof RoutingAdminError && e.code === 'cancelled'
  )
})
test('publication retry retains exact digest and operation id', async () => {
  const current = scope(),
    operation = 'f'.repeat(64),
    bodies: unknown[] = []
  api.post = (async (url, body) => {
    assert.equal(url, '/api/core/routing-admin/commit')
    bodies.push(body)
    return {
      data: {
        success: true,
        data: {
          operation_id: operation,
          published: preview.next,
          applied_on_this_node: true,
          replayed: bodies.length > 1,
        },
      },
    }
  }) as typeof api.post
  assert.equal((await publish(current, preview, operation)).replayed, false)
  assert.equal((await publish(current, preview, operation)).replayed, true)
  assert.deepEqual(bodies[0], bodies[1])
  assert.deepEqual(bodies[0], {
    preview_id: preview.preview_id,
    confirmation_sha256: preview.confirmation_sha256,
    operation_id: operation,
  })
})
test('different returned release is not confirmed', async () => {
  const current = scope(),
    operation = 'f'.repeat(64)
  api.post = (async () => ({
    data: {
      success: true,
      data: {
        operation_id: operation,
        published: { ...preview.next, routing: '7' },
        applied_on_this_node: true,
        replayed: false,
      },
    },
  })) as typeof api.post
  await assert.rejects(
    publish(current, preview, operation),
    (e) =>
      e instanceof RoutingAdminError && e.code === 'routing_admin_unavailable'
  )
})
