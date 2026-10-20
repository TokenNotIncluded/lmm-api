/*
Copyright (C) 2026 LIghtJUNction
*/
// @ts-expect-error Bun provides this module only in the test runtime.
import { mock, spyOn } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { IDBFactory } from 'fake-indexeddb'
import { Window } from 'happy-dom'

const domWindow = new Window({
  url: 'https://console.example.test/drawing',
  width: 390,
  height: 844,
})
Object.defineProperty(domWindow.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'HTMLSelectElement',
  'HTMLTextAreaElement',
  'SVGElement',
  'customElements',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const matchMedia = ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => undefined,
  removeListener: () => undefined,
  addEventListener: () => undefined,
  removeEventListener: () => undefined,
  dispatchEvent: () => false,
})) as unknown as typeof domWindow.matchMedia
Object.defineProperty(domWindow, 'matchMedia', {
  configurable: true,
  value: matchMedia,
})
Object.defineProperty(globalThis, 'matchMedia', {
  configurable: true,
  value: matchMedia,
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createRouter, createRootRoute, createMemoryHistory, RouterProvider } =
  await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { Drawing } = await import('./index')
const { marketAPI } = await import('../tool-market/api')
const { readDrawingOperation } = await import('./native-operation')
const { useAuthStore } = await import('@/stores/auth-store')
const { createDrawingHistoryStore } = await import('./history-storage')
const { resetDrawingTaskState } = await import('./drawing-task-state')
const { useSystemConfigStore } = await import('@/stores/system-config-store')
const originalCurrency = useSystemConfigStore.getState().config.currency
const originalFetch = globalThis.fetch
const originalConfirm = window.confirm

const originalGet = api.get
const originalPost = api.post
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}

// Finish each act before checking the DOM: a single outer act can defer React's
// commit until after the predicate times out. Poll observable state, bounded by
// the same attempt budget as the other asynchronous workbench checks.
async function waitForDrawingState(
  condition: () => boolean | Promise<boolean>,
  failureMessage: string
) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    await act(flushEffects)
    let ready = false
    await act(async () => {
      ready = await condition()
    })
    if (ready) return
  }
  throw new Error(failureMessage)
}

async function setTextareaValue(textarea: HTMLTextAreaElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)
  await act(async () => {
    setValue.call(textarea, value)
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
    await flushEffects()
  })
}

async function renderDrawing() {
  const router = createRouter({
    routeTree: createRootRoute({ component: Drawing }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await router.load()
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await flushEffects()
  })
  await act(flushEffects)
  return { container, queryClient, root }
}

beforeEach(() => {
  useSystemConfigStore.getState().setConfig({
    currency: {
      ...originalCurrency,
      currencyUnit: 'credit',
      creditsPerUsd: 500_000,
      creditsPerUsdExact: '500000',
      cnyPerUsd: 7,
      cnyPerUsdExact: '7',
      legacyPricingUnitsPerUsd: 1,
      quotaPerUnit: 500_000,
    },
  })
  resetDrawingTaskState()
  useAuthStore
    .getState()
    .auth.setUser({ id: 1, username: 'drawing-user', role: 10 })
  Object.defineProperty(globalThis, 'indexedDB', {
    configurable: true,
    writable: true,
    value: new IDBFactory(),
  })
  globalThis.fetch = (() => {
    throw new Error('Unexpected network fetch in drawing tests')
  }) as typeof fetch
  window.confirm = () => true
})

afterEach(() => {
  mock.restore()
  useSystemConfigStore.getState().setConfig({ currency: originalCurrency })
  resetDrawingTaskState()
  globalThis.fetch = originalFetch
  window.confirm = originalConfirm
  useAuthStore.getState().auth.reset()
  api.get = originalGet
  api.post = originalPost
  document.body.replaceChildren()
})

after(() => domWindow.close())

const toolID = '23ff32df-37fb-5105-9063-db1228dd3d3b'
const versionID = '37ff72e9-6f8e-5e34-a8e1-988253f20d27'
const callID = 'b'.repeat(64)
const grantID = 'd9c5ced2-a44e-4f2c-9519-c49f71bbbc6a'
const digest = 'a'.repeat(64)
const imageID = `image:${digest}`
const previewExpiresAt = Math.floor(Date.now() / 1000) + 100
const nativePricing = {
  schema_version: 1,
  visibility: 'session',
  observed_at: 1,
  credits_per_usd: 500000,
  rate_basis: 'effective_group_rate',
  invocation_authorized: false,
  routing_version: 1,
  pricing_version: 1,
  image_pricing_version: 1,
  groups: ['mobile-image-group'],
  models: [
    {
      id: 'images',
      model: 'image-2',
      group: 'mobile-image-group',
      apis: ['openai-images-generations', 'openai-images-edits'],
      billing_status: 'configured',
      token_price: null,
      image_variants: ['generations', 'edits'].map((endpoint) => ({
        endpoint,
        size: '1024x1024',
        quality: 'standard',
        max_n: 2,
        count_basis: 'requested',
        usd_micros_per_image: { numerator: '10', denominator: '1' },
        credits_per_image: { numerator: '5', denominator: '1' },
        reservation_credits_for_one: 5,
        rounding: 'ceil_total_credits',
        pricing_version: 1,
      })),
    },
  ],
}
const tool = {
  tool_id: toolID,
  version_id: versionID,
  name: 'drawing.generate',
  description: 'Draw',
  input_schema: '{}',
  output_schema: '{}',
  permissions: '[]',
  price_quota: 0,
}
const pendingReceipt = () => ({
  call: {
    id: callID,
    user_id: 1,
    grant_id: grantID,
    tool_id: toolID,
    version_id: versionID,
    client_id: 'web-market',
    execution_status: 'awaiting_confirmation',
    settlement_status: 'reserved',
    created_at: 1,
    resolve_by: 2000000000,
  },
  result_expired: false,
  request_state: digest,
  awaiting_confirmation: true,
  result: {
    isError: false,
    structuredContent: {
      confirmation_url: `https://console.example.test/api/user/auth/images/confirm/${imageID}`,
      billing_pending: true,
      preview: { expires_at: previewExpiresAt },
    },
  },
})
const settledReceipt = () => ({
  ...pendingReceipt(),
  call: {
    ...pendingReceipt().call,
    execution_status: 'succeeded',
    settlement_status: 'settled',
  },
  awaiting_confirmation: false,
  result: {
    isError: false,
    structuredContent: {
      operation_id: imageID,
      state: 'completed',
      charged_credits: 5,
      billing_pending: false,
      output: {
        data: [
          {
            b64_json:
              'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+y3m8AAAAASUVORK5CYII=',
          },
        ],
      },
    },
  },
})

function setupNative() {
  spyOn(api, 'get').mockImplementation(async (url: string) => {
    assert.equal(url, '/core/v1/pricing/session')
    return { data: nativePricing }
  })
  spyOn(marketAPI, 'list').mockResolvedValue([
    {
      id: 'drawing',
      owner_id: 1,
      version_id: versionID,
      name: 'Drawing',
      description: 'Draw',
      execution_type: 'builtin',
    },
  ])
  spyOn(marketAPI, 'detail').mockResolvedValue({
    service: {
      id: 'drawing',
      owner_id: 1,
      live_version_id: versionID,
      draft_version_id: '',
      status: 'published',
      created_at: 1,
    },
    version: {
      id: versionID,
      name: 'Drawing',
      description: 'Draw',
      endpoint: '',
      execution_type: 'builtin',
      visibility: 'public',
      status: 'published',
      review_note: '',
    },
    tools: [tool],
    pricing: 'free',
    validated: true,
  })
  spyOn(marketAPI, 'grant').mockResolvedValue({
    id: grantID,
    client_id: 'web-market',
    tool_id: toolID,
    version_id: versionID,
    max_price_quota: 0,
    max_total_quota: 0,
    max_calls: 1,
    reserved_quota: 0,
    spent_quota: 0,
    successful_calls: 0,
    reserved_calls: 0,
    expires_at: 2000000000,
    revoked_at: 0,
  })
  spyOn(marketAPI, 'install').mockResolvedValue(null)
  const invoke = spyOn(marketAPI, 'invoke').mockResolvedValue(pendingReceipt())
  const query = spyOn(marketAPI, 'result').mockResolvedValue(pendingReceipt())
  const lookup = spyOn(marketAPI, 'lookupRequest', async () => pendingReceipt())
  const resume = spyOn(marketAPI, 'resume').mockResolvedValue(settledReceipt())
  return { invoke, query, lookup, resume }
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll('button')).find((item) =>
    item.textContent?.includes(label)
  )
  assert.ok(found, `missing button ${label}`)
  return found
}
async function ready(rendered: Awaited<ReturnType<typeof renderDrawing>>) {
  await waitForDrawingState(
    () => !!rendered.container.querySelector('textarea'),
    'image model controls did not load'
  )
}
async function submit(rendered: Awaited<ReturnType<typeof renderDrawing>>) {
  const textarea = rendered.container.querySelector('textarea')
  assert.ok(textarea)
  await setTextareaValue(textarea, 'A mountain lake')
  await act(async () => button(rendered.container, 'Generate image').click())
  await waitForDrawingState(
    () => !!readDrawingOperation(1)?.callId,
    'preview receipt not saved'
  )
}
async function dispose(rendered: Awaited<ReturnType<typeof renderDrawing>>) {
  await act(async () => rendered.root.unmount())
  rendered.queryClient.clear()
}

test('uses the native session catalog and preserves full width controls and explicit Keys links', async () => {
  setupNative()
  const rendered = await renderDrawing()
  try {
    await ready(rendered)
    const selects = rendered.container.querySelectorAll('select')
    assert.equal(selects.length, 5)
    for (const select of selects) {
      assert.match(
        select.closest('[data-slot="native-select-wrapper"]')?.className ?? '',
        /w-full/
      )
    }
    assert.deepEqual(
      Array.from(
        rendered.container.querySelectorAll('#drawing-count option')
      ).map((option) => option.getAttribute('value')),
      ['1', '2']
    )
    const keys = Array.from(rendered.container.querySelectorAll('a')).filter(
      (link) => link.getAttribute('href') === '/keys'
    )
    assert.ok(keys.length > 0)
    assert.match(
      rendered.container.textContent ?? '',
      /Review and confirm the image price/
    )
  } finally {
    await dispose(rendered)
  }
})

test('preview does not execute: links Core confirmation and resumes the original identity only after an explicit action', async () => {
  const calls = setupNative()
  const rendered = await renderDrawing()
  try {
    await ready(rendered)
    await submit(rendered)
    assert.equal(calls.invoke.mock.calls.length, 1)
    assert.equal(calls.resume.mock.calls.length, 0)
    assert.equal(button(rendered.container, 'Generate image').disabled, true)
    assert.equal(
      rendered.container
        .querySelector('a[href*="/api/user/auth/images/confirm/"]')
        ?.getAttribute('target'),
      '_blank'
    )
    await act(async () =>
      button(rendered.container, 'Check the original request').click()
    )
    assert.equal(calls.query.mock.calls.length, 1)
    assert.equal(calls.resume.mock.calls.length, 0)
    await act(async () =>
      button(rendered.container, 'Continue the original request').click()
    )
    await waitForDrawingState(
      () => readDrawingOperation(1) === null,
      'settled result not consumed'
    )
    assert.equal(calls.resume.mock.calls.length, 1)
    assert.equal(calls.invoke.mock.calls.length, 1)
    assert.deepEqual(calls.resume.mock.calls[0], [
      callID,
      { request_state: digest },
    ])
  } finally {
    await dispose(rendered)
  }
})

test('lost preview acknowledgement survives remount and queries the same request without invoking again', async () => {
  const calls = setupNative()
  calls.invoke.mockImplementation(async () => {
    throw new Error('Connection lost')
  })
  let rendered = await renderDrawing()
  try {
    await ready(rendered)
    const textarea = rendered.container.querySelector('textarea')
    assert.ok(textarea)
    await setTextareaValue(textarea, 'A mountain lake')
    await act(async () => button(rendered.container, 'Generate image').click())
    await waitForDrawingState(
      () => !!readDrawingOperation(1),
      'original operation missing'
    )
    const requestID = readDrawingOperation(1)?.requestId
    await dispose(rendered)
    rendered = await renderDrawing()
    await ready(rendered)
    assert.equal(button(rendered.container, 'Generate image').disabled, true)
    await act(async () =>
      button(rendered.container, 'Check the original request').click()
    )
    assert.equal(calls.invoke.mock.calls.length, 1)
    assert.deepEqual(calls.lookup.mock.calls[0], [requestID])
    assert.equal(calls.resume.mock.calls.length, 0)
  } finally {
    await dispose(rendered)
  }
})

test('UNKNOWN keeps the original request locked with query as the only action', async () => {
  const calls = setupNative()
  calls.query.mockImplementation(async () => ({
    ...pendingReceipt(),
    call: { ...pendingReceipt().call, execution_status: 'unknown' },
  }))
  const rendered = await renderDrawing()
  try {
    await ready(rendered)
    await submit(rendered)
    const requestID = readDrawingOperation(1)?.requestId
    await act(async () =>
      button(rendered.container, 'Check the original request').click()
    )
    assert.equal(readDrawingOperation(1)?.requestId, requestID)
    assert.equal(button(rendered.container, 'Generate image').disabled, true)
    assert.equal(calls.invoke.mock.calls.length, 1)
    assert.equal(
      Array.from(rendered.container.querySelectorAll('button')).some((item) =>
        item.textContent?.includes('Continue the original request')
      ),
      false
    )
    assert.equal(calls.resume.mock.calls.length, 0)
  } finally {
    await dispose(rendered)
  }
})

test('a mismatched receipt cannot unlock the original request or write history', async () => {
  const calls = setupNative()
  calls.resume.mockImplementation(async () => ({
    ...settledReceipt(),
    call: { ...settledReceipt().call, grant_id: 'different' },
  }))
  const rendered = await renderDrawing()
  try {
    await ready(rendered)
    await submit(rendered)
    await act(async () =>
      button(rendered.container, 'Continue the original request').click()
    )
    assert.ok(readDrawingOperation(1))
    const history = createDrawingHistoryStore()
    assert.equal((await history.load(1)).images.length, 0)
    assert.equal(calls.invoke.mock.calls.length, 1)
  } finally {
    await dispose(rendered)
  }
})

test('reference edits send ordered bytes and their frozen metadata through the same market preview flow', async () => {
  const calls = setupNative()
  let submitted: FormData | undefined
  spyOn(api, 'post').mockImplementation(async (url: string, body: FormData) => {
    assert.equal(url, '/api/tool-market/drawing/invoke')
    submitted = body
    return {
      status: 409,
      data: {
        success: false,
        message: 'Pending',
        code: 'TOOL_MARKET_EXECUTION_PENDING',
        data: {
          ...pendingReceipt(),
          error_code: 'TOOL_MARKET_EXECUTION_PENDING',
        },
      },
    }
  })
  const rendered = await renderDrawing()
  try {
    await ready(rendered)
    const input = rendered.container.querySelector('#drawing-reference-images')
    assert.ok(input)
    const reference = new File(['abc'], 'reference.png', { type: 'image/png' })
    Object.defineProperty(input, 'files', {
      configurable: true,
      value: [reference],
    })
    await act(async () =>
      input.dispatchEvent(new Event('change', { bubbles: true }))
    )
    const textarea = rendered.container.querySelector('textarea')
    assert.ok(textarea)
    await setTextareaValue(textarea, 'Repaint the reference')
    await act(async () => button(rendered.container, 'Edit image').click())
    await waitForDrawingState(
      () => !!readDrawingOperation(1)?.callId,
      'edit preview did not preserve operation'
    )
    assert.ok(submitted)
    const request = submitted.get('request')
    assert.equal(typeof request, 'string')
    assert.ok(typeof request === 'string')
    assert.deepEqual(JSON.parse(request).arguments.reference_images, [
      {
        media_type: 'image/png',
        byte_length: 3,
        sha256:
          'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
      },
    ])
    assert.equal(submitted.getAll('images').length, 1)
    assert.equal(calls.invoke.mock.calls.length, 0)
    assert.equal(calls.resume.mock.calls.length, 0)
  } finally {
    await dispose(rendered)
  }
})

test('an account switch cannot show or save the previous account late preview', async () => {
  const calls = setupNative()
  let finish: ((value: ReturnType<typeof pendingReceipt>) => void) | undefined
  calls.invoke.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve
      })
  )
  const rendered = await renderDrawing()
  try {
    await ready(rendered)
    const textarea = rendered.container.querySelector('textarea')
    assert.ok(textarea)
    await setTextareaValue(textarea, 'Private prompt from user one')
    await act(async () => button(rendered.container, 'Generate image').click())
    await waitForDrawingState(
      () => !!readDrawingOperation(1),
      'request was not saved before dispatch'
    )
    await act(async () => {
      useAuthStore
        .getState()
        .auth.setUser({ id: 2, username: 'other-user', role: 10 })
      await flushEffects()
    })
    assert.ok(finish)
    await act(async () => {
      finish?.(pendingReceipt())
      await flushEffects()
    })
    assert.equal(readDrawingOperation(1)?.callId, undefined)
    assert.equal(readDrawingOperation(2), null)
    assert.doesNotMatch(
      rendered.container.textContent ?? '',
      /Open image confirmation/
    )
    assert.equal((await createDrawingHistoryStore().load(1)).images.length, 0)
  } finally {
    await dispose(rendered)
  }
})

test('an expired settled result is reported without automatically generating again', async () => {
  const calls = setupNative()
  calls.query.mockResolvedValue({
    ...pendingReceipt(),
    call: {
      ...pendingReceipt().call,
      execution_status: 'succeeded',
      settlement_status: 'settled',
    },
    result_expired: true,
    result: undefined,
    awaiting_confirmation: false,
  })
  const rendered = await renderDrawing()
  try {
    await ready(rendered)
    await submit(rendered)
    await act(async () =>
      button(rendered.container, 'Check the original request').click()
    )
    await waitForDrawingState(
      () => readDrawingOperation(1) === null,
      'terminal expired operation remained locked'
    )
    assert.match(
      rendered.container.textContent ?? '',
      /saved result has expired/
    )
    assert.equal(calls.invoke.mock.calls.length, 1)
    assert.equal(calls.resume.mock.calls.length, 0)
    assert.equal(
      Array.from(rendered.container.querySelectorAll('button')).some(
        (item) => item.textContent === 'Retry'
      ),
      false
    )
  } finally {
    await dispose(rendered)
  }
})
