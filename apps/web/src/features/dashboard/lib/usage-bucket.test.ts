/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { parseUsageBuckets, usageBucketSchema } from './usage-bucket'

const imageBucket = {
  user_id: 3,
  model_name: 'image-model',
  created_at: 1_700_000_000,
  count: 1,
  token_used: null,
  output_image_count: 2,
  token_count_scope: 'settled_text_requests',
  usage_scope: 'actor_personal_usage',
  settlement_scope: 'settled_requests',
  platform_sponsored: false,
  payer_kind: 'personal',
  subscription_funded: false,
  quota: 100,
  gross_settled_credits: 120,
  refunded_credits: 20,
  net_consumption_credits: 100,
  quota_usd: '0.000200',
  quota_money_available: true,
  quota_money_basis: 'captured_credits_per_usd',
  billing_basis: 'net_consumption',
  billing_currency: 'USD',
  wallet_debit_supported: false,
}

test('native images preserve absence of tokens and exact consumption metadata', () => {
  assert.deepEqual(parseUsageBuckets([imageBucket]), [imageBucket])
})

test('administrator native buckets preserve their all-accounts scope', () => {
  const bucket = { ...imageBucket, usage_scope: 'administrator_all_accounts' }
  assert.deepEqual(parseUsageBuckets([bucket]), [bucket])
})

test('mixed and zero-token native text buckets keep their known text token values', () => {
  for (const token_used of [0, 13]) {
    const value = { ...imageBucket, token_used }
    assert.equal(usageBucketSchema.parse(value).token_used, token_used)
  }
  assert.equal(
    usageBucketSchema.parse({ ...imageBucket, platform_sponsored: true })
      .platform_sponsored,
    true
  )
})

test('incomplete or inconsistent native receipts cannot fall back to legacy buckets', () => {
  for (const value of [
    { ...imageBucket, token_used: undefined },
    { ...imageBucket, token_count_scope: undefined },
    { ...imageBucket, quota: null },
    { ...imageBucket, quota_money_available: false },
    { ...imageBucket, net_consumption_credits: 99 },
    { ...imageBucket, output_image_count: 0 },
    { ...imageBucket, output_image_count: null },
    { ...imageBucket, wallet_debit_supported: true },
    { ...imageBucket, settlement_scope: 'unknown' },
  ]) {
    assert.equal(usageBucketSchema.safeParse(value).success, false)
  }
})

test('legacy buckets keep unavailable metadata absent and never acquire zero tokens', () => {
  assert.deepEqual(parseUsageBuckets([{ created_at: 1, quota: 7 }]), [
    { created_at: 1, quota: 7 },
  ])
  assert.equal(
    usageBucketSchema.parse({ created_at: 1, token_used: null }).token_used,
    null
  )
  assert.equal(
    usageBucketSchema.parse({ created_at: 1, quota: null }).quota,
    null
  )
  for (const value of [null, {}, '[]', [{ created_at: '1' }]]) {
    assert.throws(() => parseUsageBuckets(value))
  }
})
