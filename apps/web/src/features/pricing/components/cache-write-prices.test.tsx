/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import en from '@/i18n/locales/en.json'
import zh from '@/i18n/locales/zh.json'

import type { PricingModel } from '../types'
import { CacheWritePrices, formatCacheWriteRate } from './cache-write-prices'

const rate = (numerator: string, denominator = '1') => ({
  numerator,
  denominator,
})
const first = {
  up_to_input_tokens: '272000',
  input: rate('10'),
  output: rate('50'),
  cache_read: rate('0'),
  cache_write: rate('25', '2'),
  cache_write_12h: rate('25'),
}
const model: PricingModel = {
  id: 1,
  model_name: 'cache-model',
  quota_type: 0,
  enable_groups: ['default'],
  group_cache_write_12h_tiers: {
    default: [
      first,
      { ...first, up_to_input_tokens: null, cache_write_12h: rate('50') },
    ],
  },
}

test('cache lifetime prices render exact rates, both context tiers, and current language', async () => {
  const i18n = createInstance()
  await i18n.init({
    lng: 'en',
    fallbackLng: 'en',
    resources: { en, zh },
    interpolation: { escapeValue: false },
  })
  const render = (value = model) =>
    renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <CacheWritePrices model={value} />
      </I18nextProvider>
    )
  const english = render()
  for (const expected of [
    'Cache write (30 minutes)',
    'Cache write (12 hours)',
    '12.5 USD',
    '25 USD',
    '50 USD',
    '0 USD',
    'Input up to 272000 tokens',
    'Input above 272000 tokens',
  ]) {
    assert.ok(english.includes(expected), expected)
  }
  assert.ok(!english.includes('undefined'))
  await i18n.changeLanguage('zh')
  const chinese = render()
  assert.ok(chinese.includes('缓存写入（12 小时）'))
  assert.ok(chinese.includes('输入不超过 272000 token'))
  assert.equal(render({ ...model, group_cache_write_12h_tiers: undefined }), '')
  assert.equal(render({ ...model, enable_groups: [] }), '')
})

test('tiny and repeating rates never display as free or rounded currency', () => {
  assert.equal(formatCacheWriteRate(rate('0')), '0 USD')
  assert.equal(
    formatCacheWriteRate(rate('1', '1000000000000000000')),
    '0.000000000000000001 USD'
  )
  assert.equal(formatCacheWriteRate(rate('1', '3')), '1/3 USD')
})
