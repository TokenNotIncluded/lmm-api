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
import { test } from 'node:test'

import {
  changedGroupRatioOptions,
  type GroupRatioOptionValues,
} from '../group-ratio-option-values'

const baseline: GroupRatioOptionValues = {
  GroupRatio: '{"default":1}',
  TopupGroupRatio: '{}',
  UserUsableGroups: '{}',
  GroupGroupRatio: '{}',
  AutoGroups: '[]',
  MaxTokenAutoGroups: 3,
  DefaultUseAutoGroup: true,
  GroupSpecialUsableGroup: '{}',
  GroupWarnings: '{}',
}

test('builds one atomic group-pricing update with mapped warning keys', () => {
  const next = {
    ...baseline,
    GroupRatio: '{"default":0.8}',
    GroupWarnings: '{"free":{"enabled":true,"confirmations":3}}',
  }

  assert.deepEqual(changedGroupRatioOptions(next, baseline), {
    GroupRatio: '{"default":0.8}',
    'group_ratio_setting.group_warnings':
      '{"free":{"enabled":true,"confirmations":3}}',
  })
})

test('does not submit unchanged group-pricing values', () => {
  assert.deepEqual(changedGroupRatioOptions(baseline, baseline), {})
})
