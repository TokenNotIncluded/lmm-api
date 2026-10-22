import assert from 'node:assert/strict'
import { test } from 'node:test'

import { catalogSchema, previewSchema } from './contract'
import { catalog, preview } from './fixtures.test-helper'
import {
  applyPriorities,
  formatRate,
  movePriority,
  priorityChanges,
} from './helpers'
const targets = catalog.routes[0].targets
const rate = (numerator: string, denominator = '1') => ({
  numerator,
  denominator,
})
test('prices preserve zero, exact fractions and integers beyond Number precision', () => {
  for (const [n, d, out] of [
    ['0', '1', '0 USD'],
    ['1', '1000000000000000000', '0.000000000000000001 USD'],
    ['1', '3', '1/3 USD'],
    ['9007199254740993', '1', '9007199254740993 USD'],
  ])
    assert.equal(formatRate(rate(n, d)), out)
  assert.equal(formatRate(rate('7', '10'), 500000, 2000000), '0.7 USD')
})
test('same-tier drop preserves weights and joins exact priority', () => {
  const changes = movePriority(targets, '2', '1', 'same')
  assert.deepEqual(changes, [{ target_id: '2', priority: 0 }])
  assert.deepEqual(
    applyPriorities(targets, changes).map((t) => t.weight),
    [1, 3]
  )
  assert.deepEqual(movePriority(targets, '1', '1', 'same'), [])
})
test('before and after respect smaller-number-first routing', () => {
  const before = applyPriorities(
    targets,
    movePriority(targets, '2', '1', 'before')
  )
  assert.ok(before[1].priority < before[0].priority)
  const after = applyPriorities(
    targets,
    movePriority(targets, '1', '2', 'after')
  )
  assert.ok(after[0].priority > after[1].priority)
})
test('no integer gap re-spaces existing tiers together without u16 overflow', () => {
  const input = [
    targets[0],
    { ...targets[1], priority: 1 },
    { ...targets[1], target_id: '3', priority: 1 },
    { ...targets[1], target_id: '4', priority: 65535 },
  ]
  const after = applyPriorities(input, movePriority(input, '4', '2', 'before'))
  assert.equal(after[1].priority, after[2].priority)
  assert.ok(
    after[0].priority < after[3].priority &&
      after[3].priority < after[1].priority
  )
  assert.ok(after.every((t) => t.priority >= 0 && t.priority <= 65535))
  assert.equal(priorityChanges(input, after).length, 3)
})
test('64-target route retains identities and same-tier relation', () => {
  const input = Array.from({ length: 64 }, (_, i) => ({
    ...targets[0],
    target_id: String(i + 1),
    priority: Math.floor(i / 2),
  }))
  const after = applyPriorities(input, movePriority(input, '64', '1', 'before'))
  assert.equal(after.length, 64)
  assert.equal(new Set(after.map((t) => t.target_id)).size, 64)
  assert.equal(after[0].priority, after[1].priority)
  assert.ok(after[63].priority < after[0].priority)
})
test('unknown and duplicate target identities are rejected', () => {
  assert.throws(() => movePriority(targets, '99', '1', 'before'))
  assert.throws(() => priorityChanges(targets, [targets[0], targets[0]]))
})
test('catalog rejects negative fractions and duplicate routes/targets', () => {
  assert.ok(catalogSchema.safeParse(catalog).success)
  const negative = structuredClone(catalog)
  negative.prices[0].prices[0].amount.numerator = '-1'
  assert.equal(catalogSchema.safeParse(negative).success, false)
  assert.equal(
    catalogSchema.safeParse({
      ...catalog,
      routes: [catalog.routes[0], catalog.routes[0]],
    }).success,
    false
  )
  assert.equal(
    catalogSchema.safeParse({
      ...catalog,
      routes: [{ ...catalog.routes[0], targets: [targets[0], targets[0]] }],
    }).success,
    false
  )
})
test('financial preview rejects different units/model and missing frozen FX', () => {
  assert.ok(previewSchema.safeParse(preview).success)
  const different = structuredClone(preview)
  different.changes[0].after.prices[0].unit = 'request'
  assert.equal(previewSchema.safeParse(different).success, false)
  const wrong = structuredClone(preview)
  wrong.changes[0].after.model = 'other'
  assert.equal(previewSchema.safeParse(wrong).success, false)
  assert.equal(
    previewSchema.safeParse({ ...preview, exchange_rate: null }).success,
    false
  )
})
test('routing advances once and pricing advances only for price operations', () => {
  assert.equal(
    previewSchema.safeParse({
      ...preview,
      next: { ...preview.next, routing: '3' },
    }).success,
    false
  )
  assert.equal(
    previewSchema.safeParse({
      ...preview,
      next: { ...preview.next, pricing: '1' },
    }).success,
    false
  )
})
