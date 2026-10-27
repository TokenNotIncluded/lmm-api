/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  ENDPOINT_TYPES,
  FILTER_ALL,
  QUOTA_TYPES,
  SORT_OPTIONS,
} from '../constants'
import type { PricingModel } from '../types'
import { filterAndSortModels, sortModels } from './filters'

function model(
  id: number,
  modelName: string,
  vendorName: string,
  enableGroups: string[]
): PricingModel {
  return {
    id,
    model_name: modelName,
    vendor_name: vendorName,
    quota_type: 0,
    model_ratio: 1,
    completion_ratio: 1,
    enable_groups: enableGroups,
  }
}

const noOtherFilters = {
  search: '',
  quotaType: QUOTA_TYPES.ALL,
  endpointType: ENDPOINT_TYPES.ALL,
  tag: FILTER_ALL,
  sortBy: SORT_OPTIONS.NAME,
}

describe('pricing model filters', () => {
  test('price sorting leaves dynamic expressions after scalar prices without using stale legacy fields', () => {
    const cheap = {
      ...model(1, 'cheap', 'Alpha', ['all']),
      quota_type: 1,
      model_price: 1,
    }
    const expensive = {
      ...model(2, 'expensive', 'Alpha', ['all']),
      quota_type: 1,
      model_price: 10,
    }
    const fixed = {
      ...model(3, 'fixed', 'Alpha', ['all']),
      billing_mode: 'tiered_expr',
      billing_expr: 'tier("a", fixed(0))',
      model_price: 0,
    }
    const token = {
      ...model(4, 'token', 'Alpha', ['all']),
      quota_type: 1,
      billing_mode: 'tiered_expr',
      billing_expr: 'tier("a", p * 2)',
      model_price: 100,
    }
    const input = [fixed, expensive, token, cheap]
    assert.deepEqual(sortModels(input, SORT_OPTIONS.PRICE_LOW), [
      cheap,
      expensive,
      fixed,
      token,
    ])
    assert.deepEqual(sortModels(input, SORT_OPTIONS.PRICE_HIGH), [
      expensive,
      cheap,
      fixed,
      token,
    ])
    assert.deepEqual(input, [fixed, expensive, token, cheap])
  })

  test('combines provider and group filters', () => {
    const models = [
      model(1, 'alpha-standard', 'Alpha', ['standard']),
      model(2, 'alpha-premium', 'Alpha', ['premium']),
      model(3, 'beta-premium', 'Beta', ['premium']),
    ]

    const result = filterAndSortModels(models, {
      ...noOtherFilters,
      vendor: 'Alpha',
      group: 'premium',
    })

    assert.deepEqual(
      result.map((item) => item.model_name),
      ['alpha-premium']
    )
  })

  test('keeps all-groups models when a specific group is selected', () => {
    const models = [
      model(1, 'alpha-premium', 'Alpha', ['premium']),
      model(2, 'alpha-shared', 'Alpha', ['all']),
      model(3, 'beta-shared', 'Beta', ['all']),
    ]

    const result = filterAndSortModels(models, {
      ...noOtherFilters,
      vendor: 'Alpha',
      group: 'premium',
    })

    assert.deepEqual(
      result.map((item) => item.model_name),
      ['alpha-premium', 'alpha-shared']
    )
  })

  test('searches supported endpoint types advertised by the search field', () => {
    const models = [
      {
        ...model(1, 'generic-model', 'Provider', ['all']),
        supported_endpoint_types: ['anthropic'],
      },
      {
        ...model(2, 'other-model', 'Provider', ['all']),
        supported_endpoint_types: ['openai'],
      },
    ]

    const result = filterAndSortModels(models, {
      ...noOtherFilters,
      search: 'anthropic',
      vendor: FILTER_ALL,
      group: FILTER_ALL,
    })

    assert.deepEqual(
      result.map((item) => item.model_name),
      ['generic-model']
    )
  })
})
