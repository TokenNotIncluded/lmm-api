/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  getExplicitAssistantNavigation,
  getAssistantPresetForIntent,
  isExplicitAssistantL1Request,
} from './assistant-intent'

describe('assistant retention actions', () => {
  test('maps actionable intents to the matching guided flow', () => {
    assert.equal(getAssistantPresetForIntent('onboarding'), 'onboarding')
    assert.equal(getAssistantPresetForIntent('plan_purchase'), 'plan')
    assert.equal(getAssistantPresetForIntent('api_key'), 'api-key')
    assert.equal(getAssistantPresetForIntent('client_setup'), 'client-setup')
    assert.equal(getAssistantPresetForIntent('cost'), 'cost')
    assert.equal(getAssistantPresetForIntent('bounty'), 'bounty')
    assert.equal(getAssistantPresetForIntent('human_support'), 'human')
  })

  test('only navigates after an explicit page request', () => {
    assert.equal(
      getExplicitAssistantNavigation('打开 API 密钥页面', 'api_key'),
      '/keys'
    )
    assert.equal(
      getExplicitAssistantNavigation('请进入排行榜看看', 'bounty'),
      '/open-source-bounties'
    )
    assert.equal(
      getExplicitAssistantNavigation('How do I create a key?', 'api_key'),
      undefined
    )
  })

  test('does not force an action for general questions', () => {
    assert.equal(getAssistantPresetForIntent('other'), undefined)
    assert.equal(getAssistantPresetForIntent(undefined), undefined)
  })

  test('only treats an explicit access request as an L1 request', () => {
    assert.equal(isExplicitAssistantL1Request('请帮我申请 L1 权限'), true)
    assert.equal(
      isExplicitAssistantL1Request('I want to apply for developer access'),
      true
    )
    assert.equal(
      isExplicitAssistantL1Request('GPT 5.6 SOL 的价格和 Hermes 配置是什么？'),
      false
    )
  })
})
