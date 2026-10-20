/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import { usageLogSchema } from '../../data/schema'
import { parseLogsPage } from '../../lib/history-response'
import { buildLogCopyText } from '../../lib/money'
import { LogRequestSummary } from '../log-request-summary'
import { LogUsageDisplay } from '../log-usage-display'

const i18n = createInstance()
await i18n.init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} } },
})
const base = {
  id: 1,
  user_id: 1,
  created_at: 1,
  type: 2,
  content: 'Completed',
  token_id: null,
}
const settledOther = {
  pricing_dimension: 'image',
  token_usage_available: false,
  output_image_count: 2,
  billing_basis: 'net_consumption',
  request_state: 'settled',
  usage_status: 'settled',
  billing_source: 'wallet',
  platform_sponsored: true,
  reserved_credits: 0,
  pricing_credits_per_usd: 500000,
  gross_settled_credits: 2000,
  refunded_credits: 0,
  net_consumption_credits: 2000,
}
const settled = {
  ...base,
  quota: 2000,
  quota_usd: '0.004',
  quota_money_available: true,
  quota_money_basis: 'captured_credits_per_usd',
  state: 'settled',
  ...settledOther,
  other: JSON.stringify(settledOther),
}

test('native image page accepts null key IDs and keeps image usage without inventing token zero', () => {
  const result = parseLogsPage(
    {
      success: true,
      data: { items: [settled], total: 1, page: 1, page_size: 20 },
    },
    'common'
  )
  if (result.kind !== 'common') assert.fail('expected common page')
  const log = result.items[0]
  assert.equal(log.quota, 2000)
  assert.equal(log.prompt_tokens, null)
  assert.equal(log.completion_tokens, null)
  assert.equal(log.token_usage_available, false)
  assert.equal(log.output_image_count, 2)
  assert.equal(log.platform_sponsored, true)
  const html = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <LogUsageDisplay log={log} other={null} />
    </I18nextProvider>
  )
  assert.match(html, /Images \(billing quantity\).*2/)
  assert.doesNotMatch(html, /0 \/ 0/)
})

test('UNKNOWN image preserves unknown money and count while reporting its separate real hold', () => {
  const other = {
    billing_basis: 'incomplete',
    request_state: 'reconcile',
    usage_status: 'incomplete',
    pricing_dimension: 'image',
    token_usage_available: false,
    output_image_count: null,
    reserved_credits: 4000,
  }
  const log = usageLogSchema.parse({
    ...base,
    type: 5,
    quota: null,
    gross_settled_credits: null,
    refunded_credits: null,
    net_consumption_credits: null,
    quota_money_available: false,
    state: 'reconcile',
    ...other,
    other: JSON.stringify(other),
  })
  assert.equal(log.quota, null)
  assert.equal(log.output_image_count, null)
  assert.equal(log.prompt_tokens, null)
  const html = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <LogRequestSummary
        log={log}
        other={{ ...other, pricing_dimension: 'image' }}
        onClose={() => {}}
      />
    </I18nextProvider>
  )
  assert.match(html, /Images \(billing quantity\).*Unknown/)
  assert.match(html, /Reserved funds.*4,000 Raw credits/)
  assert.doesNotMatch(html, />0 USD|>0 Raw credits|0 \/ 0/)
  assert.doesNotMatch(html, /Copy safe error details/)
})

test('settled text zero stays measured zero and copied sponsored cost is not labeled Wallet', () => {
  const text = usageLogSchema.parse({
    ...base,
    quota: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    pricing_dimension: 'text',
    token_usage_available: true,
  })
  const html = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <LogUsageDisplay log={text} other={null} />
    </I18nextProvider>
  )
  assert.match(html, /0.*\/.*0/)
  const copy = buildLogCopyText(
    '',
    usageLogSchema.parse(settled),
    { ...settledOther, pricing_dimension: 'image' },
    {
      currency: 'USD',
      config: {},
      formatQuota: String,
      formatUSD: (value) => `${value} USD`,
    },
    (key) => key
  )
  assert.match(copy, /Net consumption: 0.004 USD/)
  assert.match(copy, /Billing Source: Sponsored/)
  assert.doesNotMatch(copy, /Billing Source: Wallet/)
})

test('malformed native image quantities are rejected instead of dropping the row', () => {
  for (const output_image_count of [-1, 0, 1.5, '2']) {
    assert.throws(() =>
      usageLogSchema.parse({ ...settled, output_image_count })
    )
  }
  assert.throws(() => usageLogSchema.parse({ ...settled, quota: '2000' }))
})

test('a verified settled native error receipt retains its real charge without granting legacy errors a USD basis', () => {
  const log = usageLogSchema.parse({ ...settled, type: 5 })
  const copy = buildLogCopyText(
    '',
    log,
    { ...settledOther, pricing_dimension: 'image' },
    {
      currency: 'USD',
      config: {},
      formatQuota: String,
      formatUSD: (value) => `${value} USD`,
    },
    (key) => key
  )
  assert.match(copy, /Net consumption: 0.004 USD/)
})
