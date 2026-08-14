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
import { after, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const platformFetch = globalThis.fetch
const domWindow = new Window({ url: 'http://127.0.0.1:4174/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'CustomEvent',
  'Event',
  'Request',
  'Response',
  'fetch',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { api } = await import('@/lib/http-client')
const { refreshAuthentication } = await import('@/lib/auth-session')
const { useAuthStore } = await import('@/stores/auth-store')
const {
  getActiveDebugPersona,
  installPersonaDebugRuntime,
  resetPersonaDebugRuntime,
  setActiveDebugPersona,
} = await import('./persona-runtime')

const originalAdapter = api.defaults.adapter

after(() => {
  api.defaults.adapter = originalAdapter
  globalThis.fetch = platformFetch
  useAuthStore.getState().auth.reset('idle')
  domWindow.close()
})

describe('persona debug runtime', () => {
  test('installs an isolated L0 bundle and switches complete personas', async () => {
    installPersonaDebugRuntime()

    assert.equal(getActiveDebugPersona(), 'l0')
    assert.equal(
      useAuthStore.getState().auth.user?.developer_access_granted,
      false
    )

    setActiveDebugPersona('admin')
    assert.equal(useAuthStore.getState().auth.user?.role, 100)
    assert.equal(useAuthStore.getState().auth.user?.trust_level_info?.level, 4)

    const status = await api.get('/api/assistant/status')
    assert.equal(status.data.data.is_root, true)
  })

  test('serves dynamic assistant starters without reaching a backend', async () => {
    installPersonaDebugRuntime()

    const presets = await api.get('/api/assistant/pre-conversation-presets')
    assert.equal(presets.data.success, true)
    assert.equal(presets.data.data.version, 'persona-fixture-v1')
    assert.equal(presets.data.data.presets.length, 4)
    assert.ok(
      presets.data.data.presets.every(
        (preset: { id?: string; prompt?: string }) =>
          Boolean(preset.id?.trim()) && Boolean(preset.prompt?.trim())
      )
    )

    const click = await api.post(
      '/api/assistant/pre-conversation-presets/models-and-pricing/click'
    )
    assert.deepEqual(click.data, { success: true, data: null })
  })

  test('returns lower-access conversation fixtures without raw secrets', async () => {
    setActiveDebugPersona('admin')
    const history = await api.get('/api/assistant/conversations', {
      params: { user_id: 1001 },
    })
    const serialized = JSON.stringify(history.data)

    assert.equal(history.data.data.conversations.length, 1)
    assert.match(serialized, /\[REDACTED:EMAIL\]/)
    assert.doesNotMatch(serialized, /@example\./)
  })

  test('applies the same hierarchy boundary to history lists and details', async () => {
    setActiveDebugPersona('l0')
    await assert.rejects(
      api.get('/api/assistant/conversations', { params: { user_id: 1002 } }),
      (error: unknown) =>
        (error as { response?: { status?: number } }).response?.status === 404
    )
    await assert.rejects(
      api.get('/api/assistant/conversations/8102'),
      (error: unknown) =>
        (error as { response?: { status?: number } }).response?.status === 404
    )

    setActiveDebugPersona('l1')
    const lowerAccess = await api.get('/api/assistant/conversations', {
      params: { user_id: 1001 },
    })
    assert.equal(lowerAccess.data.data.conversations.length, 1)
    assert.equal(
      (await api.get('/api/assistant/conversations/8101')).status,
      200
    )
    await assert.rejects(
      api.get('/api/assistant/conversations', { params: { user_id: 1099 } }),
      (error: unknown) =>
        (error as { response?: { status?: number } }).response?.status === 404
    )

    setActiveDebugPersona('admin')
    assert.equal(
      (
        await api.get('/api/assistant/conversations', {
          params: { user_id: 1001 },
        })
      ).status,
      200
    )
  })

  test('keeps auth refresh on the isolated adapter', async () => {
    setActiveDebugPersona('l1')
    useAuthStore.getState().auth.reset('idle')
    const outcome = await refreshAuthentication()

    assert.equal(outcome.kind, 'authenticated')
    assert.equal(useAuthStore.getState().auth.session?.sid, 'debug-persona-l1')
  })

  test('blocks unmocked axios and fetch API traffic', async () => {
    await assert.rejects(
      api.post('/api/unsafe-production-mutation', { enabled: true }),
      /PERSONA_DEBUG_UNMOCKED_REQUEST/
    )
    await assert.rejects(
      fetch('/api/unsafe-production-mutation'),
      /PERSONA_DEBUG_UNMOCKED_REQUEST/
    )
    await assert.rejects(fetch('/api'), /PERSONA_DEBUG_UNMOCKED_REQUEST/)
    await assert.rejects(fetch('/mj/task'), /PERSONA_DEBUG_UNMOCKED_REQUEST/)
    await assert.rejects(fetch('/pg/task'), /PERSONA_DEBUG_UNMOCKED_REQUEST/)
    await assert.rejects(
      fetch('https://example.com/api/status'),
      /PERSONA_DEBUG_EXTERNAL_REQUEST/
    )

    const status = await fetch('/api/status')
    assert.equal(status.status, 200)
    assert.equal((await status.json()).data.system_name, 'LMM Persona Lab')
  })

  test('reset restores the deterministic L0 fixture', () => {
    setActiveDebugPersona('l1')
    resetPersonaDebugRuntime()

    assert.equal(getActiveDebugPersona(), 'l0')
    assert.equal(useAuthStore.getState().auth.user?.id, 1001)
  })
})
