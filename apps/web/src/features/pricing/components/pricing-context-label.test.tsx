/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { Window } from 'happy-dom'
import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider, initReactI18next } from 'react-i18next'

import {
  getAccountPriceMultiplier,
  getPricingBasisLabel,
} from '../lib/pricing-context'
import type { PricingModel } from '../types'
import { ImagePrices } from './image-prices'
import { ModelCard } from './model-card'
import { ModelDetailsContent } from './model-details'
import { PricingContextLabel } from './pricing-context-label'
import { PricingToolbar, type PricingToolbarProps } from './pricing-toolbar'
import { RequestEstimator } from './request-estimator'

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const domWindow = new Window()
const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
})
after(() => {
  queryClient.clear()
  domWindow.close()
})

const accountModel: PricingModel = {
  id: 1,
  model_name: 'account-priced-model',
  quota_type: 0,
  pricing_schema_version: 2,
  pricing_currency: 'USD',
  input_price: 1.94,
  output_price: 3.88,
  cache_read_price: null,
  enable_groups: ['standard'],
  group_ratio: { standard: 10 },
  pricing_context: {
    rate_basis: 'effective_user_rate',
    user_price: {
      status: 'verified',
      multiplier: { numerator: '97', denominator: '100' },
    },
  },
}

const baseImageVariant = {
  endpoint: 'generations',
  size: '1024x1024',
  quality: 'standard',
  max_n: 1,
  count_basis: 'requested',
  usd_micros_per_image: { numerator: '40000', denominator: '1' },
  credits_per_image: { numerator: '20000', denominator: '1' },
  reservation_credits_for_one: 20000,
  rounding: 'ceil_total_credits',
  pricing_version: 1,
  group: 'standard',
  model: 'image-model',
  rate_basis: 'base_group_rate',
} as const

function quoteModel(numerator: string, denominator: string): PricingModel {
  return {
    ...accountModel,
    pricing_context: {
      rate_basis: 'effective_user_rate',
      user_price: {
        status: 'verified',
        multiplier: { numerator, denominator },
      },
    },
  }
}

function render(element: React.ReactNode) {
  const container = domWindow.document.createElement('div')
  container.innerHTML = renderToStaticMarkup(
    <QueryClientProvider client={queryClient}>
      <I18nextProvider i18n={i18n}>{element}</I18nextProvider>
    </QueryClientProvider>
  )
  return container
}

function renderCard(model: PricingModel) {
  return render(<ModelCard model={model} onClick={() => {}} />)
}

function renderEstimator(model: PricingModel) {
  return render(
    <RequestEstimator
      model={model}
      groupRatio={{ standard: 10 }}
      usableGroup={{ standard: { desc: 'Standard', ratio: 10 } }}
      endpointMap={{}}
      autoGroups={[]}
      tokenUnit='M'
      displayCurrency='USD'
    />
  )
}

for (const [numerator, denominator, expected] of [
  ['97', '100', '97%'],
  ['1', '1', '100%'],
  ['1', '8', '12.5%'],
  ['1', '3', '1/3×'],
  ['999999999999999999', '1000000000000000000', '99.9999999999999999%'],
] as const) {
  test(`account factor ${numerator}/${denominator} displays without rounding`, () => {
    assert.equal(
      getAccountPriceMultiplier(quoteModel(numerator, denominator)),
      expected
    )
  })
}

test('public group prices do not invent a personal discount', () => {
  const model: PricingModel = {
    ...accountModel,
    pricing_context: {
      rate_basis: 'base_group_rate',
      user_price: { status: 'not_evaluated' },
    },
  }
  const text = renderCard(model).textContent ?? ''
  assert.match(text, /Group base price/)
  assert.doesNotMatch(
    text,
    /Current account price|Account discount|100%|not_evaluated|verified/
  )
  assert.equal(getAccountPriceMultiplier(model), null)
})

test('the account label explains the multiplier and already discounted rates', () => {
  const text =
    render(<PricingContextLabel model={accountModel} showDiscountNote />)
      .textContent ?? ''
  assert.match(text, /Current account price/)
  assert.match(text, /Account discount: 97% of group base price/)
  assert.match(text, /Prices include your account discount\./)
  assert.doesNotMatch(text, /verified|trust|proof|source.policy/i)
})

test('the card uses returned account rates without multiplying either factor again', () => {
  for (const model of [accountModel, quoteModel('1', '2')]) {
    const text = renderCard(model).textContent ?? ''
    assert.match(text, /Input 1\.94 USD/)
    assert.match(text, /Output 3\.88 USD/)
    assert.doesNotMatch(text, /1\.8818|0\.97 USD|19\.4 USD/)
  }
})

test('a missing account input rate stays unavailable rather than becoming free', () => {
  const model = { ...accountModel, input_price: null }
  const text = renderCard(model).textContent ?? ''
  assert.match(text, /Input -/)
  assert.doesNotMatch(text, /Input 0 USD/)
  assert.match(renderEstimator(model).textContent ?? '', /Estimate unavailable/)
})

test('request estimates use returned account rates once and describe that basis', () => {
  for (const model of [accountModel, quoteModel('1', '2')]) {
    const estimate = renderEstimator(model)
    assert.equal(
      estimate.querySelector('[aria-live="polite"]')?.textContent,
      '0.02716 USD'
    )
    assert.match(estimate.textContent ?? '', /Current account price/)
    assert.match(
      estimate.textContent ?? '',
      /One request using the displayed prices/
    )
    assert.doesNotMatch(estimate.textContent ?? '', /base price \(1×\)/)
  }
})

test('legacy pricing keeps its existing basis label and has no guessed discount', () => {
  for (const pricing_context of [
    undefined,
    { rate_basis: 'effective_group_rate' } as const,
  ]) {
    const model = { ...accountModel, pricing_context }
    assert.equal(getPricingBasisLabel(model), 'Base price (1×)')
    assert.equal(getAccountPriceMultiplier(model), null)
    assert.doesNotMatch(
      renderCard(model).textContent ?? '',
      /Account discount|Current account price/
    )
  }
})

test('image-only pricing uses its own explicit basis instead of token account context', () => {
  const imageModel: PricingModel = {
    ...accountModel,
    quota_type: 2,
    image_variants: [baseImageVariant],
  }
  assert.equal(getPricingBasisLabel(imageModel), 'Group base price')
  assert.equal(getAccountPriceMultiplier(imageModel), null)
  assert.doesNotMatch(
    renderCard(imageModel).textContent ?? '',
    /Account discount|Current account price/
  )
  const accountImages: PricingModel = {
    ...imageModel,
    image_variants: [
      { ...baseImageVariant, rate_basis: 'effective_user_rate' },
    ],
  }
  assert.equal(getPricingBasisLabel(accountImages), 'Current account price')
  assert.equal(getAccountPriceMultiplier(accountImages), '97%')
  assert.equal(
    getAccountPriceMultiplier({ ...imageModel, image_variants: [] }),
    null
  )
  assert.equal(
    getPricingBasisLabel({ ...imageModel, image_variants: [] }),
    'Price'
  )
  assert.equal(
    getPricingBasisLabel({
      ...imageModel,
      image_variants: [
        baseImageVariant,
        { ...baseImageVariant, rate_basis: 'effective_user_rate' },
      ],
    }),
    'Image prices'
  )
})

test('mixed token and image prices label each image basis and do not discount its rates again', () => {
  const model: PricingModel = {
    ...accountModel,
    image_variants: [
      baseImageVariant,
      {
        ...baseImageVariant,
        rate_basis: 'effective_user_rate',
        usd_micros_per_image: { numerator: '38800', denominator: '1' },
        credits_per_image: { numerator: '19400', denominator: '1' },
      },
    ],
  }
  const prices = render(<ImagePrices model={model} />)
  const lists = [...prices.querySelectorAll('dl')]
  assert.equal(lists.length, 2)
  assert.match(lists[0].textContent ?? '', /Group base price0\.04 USD/)
  assert.match(lists[0].textContent ?? '', /20000 Credits/)
  assert.doesNotMatch(
    lists[0].textContent ?? '',
    /Current account price|Account discount/
  )
  assert.match(lists[1].textContent ?? '', /Current account price0\.0388 USD/)
  assert.match(lists[1].textContent ?? '', /19400 Credits/)
  assert.doesNotMatch(prices.textContent ?? '', /0\.037636|18818 Credits/)
})

test('model details show already discounted account group prices and keep missing rates unavailable', () => {
  const model: PricingModel = {
    ...accountModel,
    performance_available: false,
    group_prices: {
      standard: {
        input_price: 1.94,
        output_price: null,
        cache_read_price: null,
        cache_write_price: null,
        image_price: null,
        audio_input_price: null,
        audio_output_price: null,
        model_price: null,
      },
    },
  }
  const details = render(
    <ModelDetailsContent
      model={model}
      groupRatio={{ standard: 10 }}
      usableGroup={{}}
      endpointMap={{}}
      autoGroups={[]}
      tokenUnit='M'
      displayCurrency='USD'
    />
  )
  const text = details.textContent ?? ''
  assert.match(text, /Current account price/)
  assert.match(text, /Account discount: 97% of group base price/)
  assert.match(text, /Prices include your account discount/)
  assert.match(text, /Input1\.94 USD/)
  assert.match(text, /Output-/)
  assert.doesNotMatch(text, /1\.8818 USD|19\.4 USD|base price \(1×\)/)
})

test('toolbar explains a shared account price without claiming an old 1x basis', () => {
  const noChange = () => {}
  const props: PricingToolbarProps = {
    filteredCount: 1,
    sortBy: 'name',
    onSortChange: noChange,
    tokenUnit: 'M',
    onTokenUnitChange: noChange,
    displayCurrency: 'USD',
    onDisplayCurrencyChange: noChange,
    viewMode: 'card',
    onViewModeChange: noChange,
    quotaTypeFilter: 'all',
    endpointTypeFilter: 'all',
    vendorFilter: 'all',
    groupFilter: 'all',
    tagFilter: 'all',
    onQuotaTypeChange: noChange,
    onEndpointTypeChange: noChange,
    onVendorChange: noChange,
    onGroupChange: noChange,
    onTagChange: noChange,
    vendors: [],
    groups: ['standard'],
    tags: [],
    models: [accountModel],
    hasActiveFilters: false,
    activeFilterCount: 0,
    onClearFilters: noChange,
  }
  const toolbar = render(<PricingToolbar {...props} />)
  const note = toolbar.querySelector('[role="note"]')?.textContent ?? ''
  assert.match(note, /Current account price/)
  assert.match(note, /Account discount: 97% of group base price/)
  assert.match(note, /Prices include your account discount/)
  assert.doesNotMatch(note, /base price \(1×\)/)
})
