/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'
import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import { QUOTA_TYPES } from '../constants'
import { filterByQuotaType, getModelQuotaType } from '../lib/filters'
import type { PricingModel } from '../types'
import { PricingSidebar } from './pricing-sidebar'

const cases: {
  name: string
  pricing: Partial<PricingModel>
  category: string | undefined
}[] = [
  { name: 'legacy token', pricing: {}, category: 'token' },
  { name: 'legacy request', pricing: { quota_type: 1 }, category: 'request' },
  { name: 'legacy image', pricing: { quota_type: 2 }, category: undefined },
  {
    name: 'fixed request',
    pricing: { billing_expr: 'tier("request", fixed(0.01))' },
    category: 'request',
  },
  {
    name: 'zero fixed request',
    pricing: { billing_expr: 'v1:tier("free", fixed(0))' },
    category: 'request',
  },
  {
    name: 'nested request tiers with request and header conditions',
    pricing: {
      billing_expr:
        'p > 0 ? (len < 1000 ? tier("short", fixed(0)) : tier("hd", fixed(0.02))) : tier("base", fixed(0.01))',
    },
    category: 'request',
  },
  {
    name: 'nested mixed tiers with time conditions are token priced',
    pricing: {
      quota_type: 1,
      billing_expr:
        'hour("UTC") < 12 ? (len < 1000 ? tier("short", fixed(0.01)) : tier("long", p * 2 + c * 8)) : tier("night", fixed(0.02))',
    },
    category: 'token',
  },
  {
    name: 'token tier overrides legacy request',
    pricing: { quota_type: 1, billing_expr: 'tier("base", p * 2 + c * 8)' },
    category: 'token',
  },
  {
    name: 'fixed syntax in label is not fixed pricing',
    pricing: { quota_type: 1, billing_expr: 'tier("fixed(0.01)", cr * 0.1)' },
    category: 'token',
  },
  {
    name: 'invalid syntax falls back to request',
    pricing: { quota_type: 1, billing_expr: 'tier("request", fixed(' },
    category: 'request',
  },
  {
    name: 'invalid syntax falls back to token',
    pricing: { billing_expr: 'tier("request", fixed(0));' },
    category: 'token',
  },
  {
    name: 'missing expression falls back',
    pricing: { quota_type: 1, billing_expr: '' },
    category: 'request',
  },
  {
    name: 'non dynamic mode ignores expression',
    pricing: {
      billing_mode: 'ratio',
      billing_expr: 'tier("request", fixed(1))',
    },
    category: 'token',
  },
  {
    name: 'upstream cost keeps its category',
    pricing: { quota_type: 3, billing_expr: 'tier("request", fixed(1))' },
    category: 'upstream_cost',
  },
  {
    name: 'tier amount in condition is not classified as output',
    pricing: {
      quota_type: 1,
      billing_expr:
        'tier("condition", p) > 0 ? tier("a", fixed(1)) : tier("b", fixed(2))',
    },
    category: 'request',
  },
  {
    name: 'direct token output branch prevents request classification',
    pricing: {
      quota_type: 1,
      billing_expr: 'len > 100 ? tier("a", fixed(1)) : p * 2',
    },
    category: 'token',
  },
  {
    name: 'fixed prices with a multiplier stay request priced',
    pricing: {
      quota_type: 0,
      billing_expr: '(tier("a", fixed(1)) * 2) / 4',
    },
    category: 'request',
  },
  {
    name: 'adding fixed tier prices remains request priced',
    pricing: {
      quota_type: 0,
      billing_expr: 'tier("a", fixed(1)) + tier("b", fixed(2))',
    },
    category: 'request',
  },
  {
    name: 'max of fixed tier prices remains request priced',
    pricing: {
      quota_type: 0,
      billing_expr: 'max(tier("a", fixed(1)), tier("b", fixed(2)))',
    },
    category: 'request',
  },
  {
    name: 'max of token tier prices stays token priced',
    pricing: {
      quota_type: 1,
      billing_expr: 'max(tier("a", p), tier("b", c))',
    },
    category: 'token',
  },
  {
    name: 'param and header conditions select fixed request tiers',
    pricing: {
      quota_type: 0,
      billing_expr:
        '(param("quality") == "hd" ? tier("hd", fixed(0.02)) : tier("base", fixed(0.01))) * (header("x-fast") == "true" ? 2 : 1)',
    },
    category: 'request',
  },
  {
    name: 'time conditions select fixed request tiers',
    pricing: {
      quota_type: 0,
      billing_expr:
        'hour("UTC") < 12 ? tier("morning", fixed(0.02)) : tier("evening", fixed(0.01))',
    },
    category: 'request',
  },
  {
    name: 'time conditions with mixed tier amounts stay token priced',
    pricing: {
      quota_type: 1,
      billing_expr:
        'hour("UTC") < 12 ? tier("morning", fixed(0.02)) : tier("evening", p * 2)',
    },
    category: 'token',
  },
  {
    name: 'direct token amount beside a fixed tier prevents request classification',
    pricing: {
      quota_type: 1,
      billing_expr: 'tier("a", fixed(1)) + c * 2',
    },
    category: 'token',
  },
  {
    name: 'invalid evaluator calls fall back to legacy category',
    pricing: {
      quota_type: 1,
      billing_expr: 'tier("a", min())',
    },
    category: 'request',
  },
  {
    name: 'string tier amount falls back to legacy category',
    pricing: {
      quota_type: 1,
      billing_expr: 'tier("a", "oops")',
    },
    category: 'request',
  },
  {
    name: 'unsupported expression version falls back to legacy category',
    pricing: {
      quota_type: 1,
      billing_expr: 'v99:tier("a", fixed(1))',
    },
    category: 'request',
  },
  {
    name: 'fixed in a condition does not turn token prices into request prices',
    pricing: {
      quota_type: 1,
      billing_expr: 'fixed(1) ? tier("a", p) : tier("b", c)',
    },
    category: 'token',
  },
  {
    name: 'no tier falls back to legacy request',
    pricing: { quota_type: 1, billing_expr: 'p * 2' },
    category: 'request',
  },
  {
    name: 'fixed without tier falls back to legacy token',
    pricing: { billing_expr: 'fixed(1)' },
    category: 'token',
  },
  {
    name: 'unknown function in unselected branch falls back',
    pricing: {
      quota_type: 1,
      billing_expr: 'true ? tier("a", p) : unsupported(1)',
    },
    category: 'request',
  },
  {
    name: 'invalid fixed arguments fall back',
    pricing: { billing_expr: 'tier("a", fixed())' },
    category: 'token',
  },
  {
    name: 'invalid tier arguments fall back',
    pricing: { quota_type: 1, billing_expr: 'tier("a")' },
    category: 'request',
  },
  {
    name: 'unknown variable falls back',
    pricing: { quota_type: 1, billing_expr: 'tier("a", unknown * 2)' },
    category: 'request',
  },
  {
    name: 'bounded parser rejection falls back',
    pricing: {
      quota_type: 1,
      billing_expr: `${'('.repeat(70)}p${')'.repeat(70)}`,
    },
    category: 'request',
  },
]

const i18n = createInstance()
await i18n.init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} } },
})

function sidebarButtons(models: PricingModel[]) {
  const ignore = () => {}
  const window = new Window()
  window.document.body.innerHTML = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <PricingSidebar
        quotaTypeFilter='all'
        endpointTypeFilter='all'
        vendorFilter='all'
        groupFilter='all'
        tagFilter='all'
        onQuotaTypeChange={ignore}
        onEndpointTypeChange={ignore}
        onVendorChange={ignore}
        onGroupChange={ignore}
        onTagChange={ignore}
        vendors={[]}
        groups={[]}
        tags={[]}
        models={models}
        hasActiveFilters={false}
        onClearFilters={ignore}
      />
    </I18nextProvider>
  )
  const buttons = [...window.document.querySelectorAll('button')].map(
    (button) => button.textContent?.replaceAll(/\s/g, '')
  )
  window.close()
  return buttons
}

const categories = [
  [QUOTA_TYPES.TOKEN, 'Token-based'],
  [QUOTA_TYPES.REQUEST, 'Per Request'],
  [QUOTA_TYPES.UPSTREAM_COST, 'Upstream cost'],
] as const

const models = cases.map(
  ({ name, pricing }, index): PricingModel => ({
    id: index,
    model_name: name,
    quota_type: 0,
    enable_groups: ['default'],
    billing_mode: 'tiered_expr',
    ...pricing,
  })
)

for (const [index, fixture] of cases.entries()) {
  test(`quota filter and sidebar agree: ${fixture.name}`, () => {
    const model = models[index]
    assert.equal(getModelQuotaType(model), fixture.category)
    const buttons = sidebarButtons([model])
    for (const [category, label] of categories) {
      const expected = category === fixture.category ? [model] : []
      assert.deepEqual(filterByQuotaType([model], category), expected)
      assert.ok(
        buttons.includes(`${label.replaceAll(/\s/g, '')}${expected.length}`)
      )
    }
    assert.deepEqual(filterByQuotaType([model], QUOTA_TYPES.ALL), [model])
    assert.ok(buttons.includes('AllModels1'))
  })
}

test('sidebar counts match filtering for a mixed catalog and updated input', () => {
  for (const catalog of [models, models.slice(3, 7), []]) {
    const buttons = sidebarButtons(catalog)
    for (const [category, label] of categories) {
      const expected = cases.filter(
        (fixture, index) =>
          catalog.includes(models[index]) && fixture.category === category
      ).length
      assert.equal(filterByQuotaType(catalog, category).length, expected)
      assert.ok(buttons.includes(`${label.replaceAll(/\s/g, '')}${expected}`))
    }
    assert.ok(buttons.includes(`AllModels${catalog.length}`))
  }
})
