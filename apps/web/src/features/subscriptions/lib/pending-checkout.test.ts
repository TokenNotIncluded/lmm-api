/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  SUBSCRIPTION_CHECKOUT_POLL_TIMEOUT_MS,
  beginSubscriptionCheckoutConfirmation,
  shouldContinueSubscriptionCheckoutConfirmation,
  subscriptionCheckoutFingerprint,
} from './pending-checkout'

describe('subscription checkout confirmation', () => {
  test('keeps polling only until a subscription record changes or expires', () => {
    const pending = beginSubscriptionCheckoutConfirmation('before', 100)

    assert.equal(
      shouldContinueSubscriptionCheckoutConfirmation(pending, 'before', 101),
      true
    )
    assert.equal(
      shouldContinueSubscriptionCheckoutConfirmation(pending, 'after', 101),
      false
    )
    assert.equal(
      shouldContinueSubscriptionCheckoutConfirmation(
        pending,
        'before',
        100 + SUBSCRIPTION_CHECKOUT_POLL_TIMEOUT_MS
      ),
      false
    )
  })

  test('uses stable subscription fields rather than response ordering', () => {
    const first = {
      subscription: {
        id: 2,
        user_id: 1,
        plan_id: 3,
        status: 'active',
        start_time: 10,
        end_time: 20,
        amount_total: 100,
        amount_used: 5,
      },
    }
    const second = {
      subscription: {
        id: 1,
        user_id: 1,
        plan_id: 2,
        status: 'active',
        start_time: 9,
        end_time: 19,
        amount_total: 50,
        amount_used: 0,
      },
    }

    assert.equal(
      subscriptionCheckoutFingerprint([first, second]),
      subscriptionCheckoutFingerprint([second, first])
    )
  })
})
