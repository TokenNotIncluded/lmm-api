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

export type GroupRatioOptionValues = {
  GroupRatio: string
  TopupGroupRatio: string
  UserUsableGroups: string
  GroupGroupRatio: string
  AutoGroups: string
  MaxTokenAutoGroups: number
  DefaultUseAutoGroup: boolean
  GroupSpecialUsableGroup: string
  GroupWarnings: string
}

const optionKeys: Partial<Record<keyof GroupRatioOptionValues, string>> = {
  GroupSpecialUsableGroup: 'group_ratio_setting.group_special_usable_group',
  GroupWarnings: 'group_ratio_setting.group_warnings',
}

// Keep the values and their option-key mapping together so group pricing is
// always written as one validated server-side transaction.
export function changedGroupRatioOptions(
  next: GroupRatioOptionValues,
  previous: GroupRatioOptionValues
) {
  const changes: Record<string, string> = {}
  for (const key of Object.keys(next) as Array<keyof GroupRatioOptionValues>) {
    if (next[key] !== previous[key]) {
      changes[optionKeys[key] ?? key] = String(next[key])
    }
  }
  return changes
}
