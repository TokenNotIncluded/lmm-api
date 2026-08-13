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
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  getDefaultWaffoPancakeCheckoutRegion,
  getWaffoPancakeCheckoutLanguage,
} from './waffo-pancake-checkout'

describe('Waffo Pancake checkout preferences', () => {
  test('uses the China region for every normalized Chinese interface locale', () => {
    for (const language of [
      'zh',
      'zhCN',
      'zh-CN',
      'zh-Hans',
      'zhTW',
      'zh-TW',
    ]) {
      assert.equal(getDefaultWaffoPancakeCheckoutRegion(language), 'china')
    }
  })

  test('uses the global region for non-Chinese interface locales', () => {
    for (const language of ['en', 'fr', 'ja', 'ru', 'vi', 'unknown']) {
      assert.equal(getDefaultWaffoPancakeCheckoutRegion(language), 'global')
    }
  })

  test('maps interface locales to Waffo checkout language values', () => {
    const expected: Record<string, string> = {
      zh: 'zh-Hans',
      zhCN: 'zh-Hans',
      zhTW: 'zh-Hant-TW',
      ja: 'ja-JP',
      ru: 'ru-RU',
      vi: 'vi-VN',
      en: 'en',
      fr: 'en',
    }

    for (const [language, checkoutLanguage] of Object.entries(expected)) {
      assert.equal(getWaffoPancakeCheckoutLanguage(language), checkoutLanguage)
    }
  })
})
