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
import { normalizeInterfaceLanguage } from '@/i18n/languages'

export type WaffoPancakeCheckoutRegion = 'china' | 'global'

export type WaffoPancakeCheckoutLanguage =
  | 'zh-Hans'
  | 'zh-Hant-TW'
  | 'ja-JP'
  | 'ru-RU'
  | 'vi-VN'
  | 'en'

export interface WaffoPancakeCheckoutOptions {
  checkout_region: WaffoPancakeCheckoutRegion
  checkout_language: WaffoPancakeCheckoutLanguage
}

/**
 * Map the interface locale to Waffo's supported checkout language enum.
 * Unknown/unsupported interface languages intentionally use Waffo English.
 */
export function getWaffoPancakeCheckoutLanguage(
  interfaceLanguage?: string | null
): WaffoPancakeCheckoutLanguage {
  switch (normalizeInterfaceLanguage(interfaceLanguage)) {
    case 'zhCN':
      return 'zh-Hans'
    case 'zhTW':
      return 'zh-Hant-TW'
    case 'ja':
      return 'ja-JP'
    case 'ru':
      return 'ru-RU'
    case 'vi':
      return 'vi-VN'
    default:
      return 'en'
  }
}

/**
 * Chinese interface languages use the China checkout by default. The caller
 * can keep an explicit override separately so changing the interface locale
 * does not reset a region the user already chose.
 */
export function getDefaultWaffoPancakeCheckoutRegion(
  interfaceLanguage?: string | null
): WaffoPancakeCheckoutRegion {
  const language = normalizeInterfaceLanguage(interfaceLanguage)
  return language === 'zhCN' || language === 'zhTW' ? 'china' : 'global'
}
