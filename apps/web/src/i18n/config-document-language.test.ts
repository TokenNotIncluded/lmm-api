/* Copyright (C) 2026 LMM API contributors. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReadCallback } from 'i18next'

const browser = new Window({ url: 'http://localhost/' })
browser.localStorage.setItem('i18nextLng', 'zhCN')
const previous = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'localStorage',
] as const) {
  previous.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: key === 'window' ? browser : browser[key],
  })
}

const { default: appI18n } = await import('./config')
if (!appI18n.isInitialized) {
  await new Promise<void>((resolve) =>
    appI18n.on('initialized', () => resolve())
  )
}

after(() => {
  for (const [key, descriptor] of previous) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
  browser.close()
})

test('initial document language follows the stored Chinese interface', () => {
  assert.equal(appI18n.resolvedLanguage, 'zhCN')
  assert.equal(appI18n.t('Home'), '主页')
  assert.equal(document.documentElement.lang, 'zh-CN')
})

test('failed locale loads declare the rendered fallback', async () => {
  const backend = appI18n.services.backendConnector.backend
  const originalRead = backend.read
  backend.read = (
    language: string,
    namespace: string,
    callback: ReadCallback
  ) => {
    if (language === 'ja') {
      callback(new Error('synthetic locale failure'), false)
    } else originalRead.call(backend, language, namespace, callback)
  }
  try {
    await appI18n.changeLanguage('ja')
    assert.equal(appI18n.resolvedLanguage, 'en')
    assert.equal(appI18n.t('Home'), 'Home')
    assert.equal(document.documentElement.lang, 'en')
  } finally {
    backend.read = originalRead
  }
  // Restore the dictionary explicitly: i18next remembers failed loads.
  const { loadLocaleResources } = await import('./config')
  appI18n.addResourceBundle(
    'ja',
    'translation',
    await loadLocaleResources('ja')
  )
})

test('switches all supported document languages and falls back for unknown codes', async () => {
  for (const [language, expected] of [
    ['zhCN', 'zh-CN'],
    ['zhTW', 'zh-TW'],
    ['en', 'en'],
    ['fr', 'fr'],
    ['ru', 'ru'],
    ['ja', 'ja'],
    ['vi', 'vi'],
  ]) {
    await appI18n.changeLanguage(language)
    assert.equal(appI18n.resolvedLanguage, language)
    assert.equal(document.documentElement.lang, expected)
  }
  await appI18n.changeLanguage('unsupported')
  assert.equal(appI18n.resolvedLanguage, 'en')
  assert.equal(document.documentElement.lang, 'en')
})
