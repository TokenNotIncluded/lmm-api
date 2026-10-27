/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window()
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: name === 'window' ? dom : dom[name],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { getNativeSystemResources } = await import('./api')
const { NativeResourcesPanel } =
  await import('./components/native-resources-panel')
const originalGet = api.get
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

const snapshot = {
  schema_version: 1,
  observed_at_unix_ms: 123456,
  sample_window_ms: 1000,
  system: {
    logical_cpus: 2,
    load_average: { one: 0.1, five: 0.2, fifteen: 0.3 },
    cpu_usage_percent: 5,
    memory_total_bytes: 2048,
    memory_available_bytes: 1024,
    memory_used_bytes: 1024,
    memory_usage_percent: 50,
  },
  processes: {
    'lmm-core': {
      status: 'running',
      version: '0.1.0',
      pid: 123,
      cpu_percent: 2,
      memory_bytes: 512,
      memory_percent: 25,
      threads: 4,
    },
    'lmm-extensions': {
      status: 'unavailable',
      version: null,
      pid: null,
      cpu_percent: null,
      memory_bytes: null,
      memory_percent: null,
      threads: null,
    },
  },
}

function respond(data: unknown) {
  api.get = (async (path) => {
    assert.equal(path, '/core/v1/admin/system-resources')
    return { data }
  }) as typeof api.get
}

let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.get = originalGet
  document.body.replaceChildren()
})
after(() => dom.close())

test('native resource response preserves real processes and accepts null samples', async () => {
  respond(snapshot)
  const actual = await getNativeSystemResources()
  assert.equal(actual.processes['lmm-core'].pid, 123)
  assert.equal(actual.processes['lmm-core'].cpu_percent, 2)
  assert.equal(actual.processes['lmm-extensions'].status, 'unavailable')
  assert.equal(actual.processes['lmm-extensions'].pid, undefined)
  assert.equal(actual.processes['lmm-extensions'].version, undefined)
})

test('native resource response rejects drift and missing process entries', async () => {
  const missingMetric = structuredClone(snapshot)
  Reflect.deleteProperty(missingMetric.processes['lmm-core'], 'pid')
  const missingProcess = structuredClone(snapshot)
  Reflect.deleteProperty(missingProcess.processes, 'lmm-extensions')
  for (const data of [
    '<html>not the endpoint</html>',
    { ...snapshot, schema_version: 2 },
    { ...snapshot, system: { ...snapshot.system, memory_used_bytes: '1024' } },
    {
      ...snapshot,
      processes: {
        core: snapshot.processes['lmm-core'],
        extensions: snapshot.processes['lmm-extensions'],
      },
    },
    missingProcess,
    missingMetric,
    { ...snapshot, processes: { ...snapshot.processes, 'lmm-core': null } },
  ]) {
    respond(data)
    await assert.rejects(
      getNativeSystemResources(),
      /Invalid system resources response/
    )
  }
})

test('invalid process data renders the existing error state', async () => {
  respond({
    ...snapshot,
    processes: { core: snapshot.processes['lmm-core'] },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <NativeResourcesPanel />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  for (let attempt = 0; attempt < 100; attempt++) {
    if (container.textContent?.includes('Invalid system resources response')) {
      break
    }
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.match(container.textContent ?? '', /Invalid system resources response/)
  assert.doesNotMatch(container.textContent ?? '', /Not reported/)
})
