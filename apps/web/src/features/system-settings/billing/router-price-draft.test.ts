/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { ROUTER_MODELS, routerPriceDraft } from './router-price-draft'

test('router configuration preview emits exact Core ppm with zero fixed rates', () => {
  for (const model of ROUTER_MODELS) {
    const draft = routerPriceDraft('7', model, 'standard', '1.234567', [
      'standard',
    ])
    assert.ok(draft)
    const rule = JSON.parse(draft)
    assert.equal(rule.group_multiplier_ppm, 1234567)
    assert.equal(rule.billing_mode, 'upstream_cost')
    assert.equal(rule.model, model)
    assert.equal(rule.speed, 'standard')
    assert.deepEqual(Object.values(rule.rates), [0, 0, 0, 0, 0])
  }
})

test('preview rejects unsafe IDs, unsupported routers, absent groups and inexact multipliers', () => {
  for (const [id, model, group, multiplier] of [
    ['0', ROUTER_MODELS[0], 'standard', '1'],
    ['9007199254740992', ROUTER_MODELS[0], 'standard', '1'],
    ['1', 'guessed-router', 'standard', '1'],
    ['1', ROUTER_MODELS[0], 'other', '1'],
    ...['0', '-1', '1000.000001', '1.0000001', '1e2', '', 'NaN'].map(
      (factor) => ['1', ROUTER_MODELS[0], 'standard', factor] as const
    ),
  ] as const) {
    assert.equal(
      routerPriceDraft(id, model, group, multiplier, ['standard']),
      null
    )
  }
})
