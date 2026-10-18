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
import i18n, { type BackendModule, type ReadCallback } from 'i18next'
import LanguageDetector from 'i18next-browser-languagedetector'
import { initReactI18next } from 'react-i18next'

import {
  convertDetectedLanguage,
  type InterfaceLanguageCode,
  normalizeInterfaceLanguage,
  toIntlLocale,
} from './languages'

type LocaleModule = {
  default: { translation: Record<string, string> }
}

type LocaleLoader = () => Promise<LocaleModule>

const localeLoaders = {
  en: () => import('./locales/en.json'),
  zhCN: () => import('./locales/zh.json'),
  fr: () => import('./locales/fr.json'),
  ru: () => import('./locales/ru.json'),
  ja: () => import('./locales/ja.json'),
  vi: () => import('./locales/vi.json'),
  zhTW: () => import('./locales/zh-TW.json'),
} satisfies Record<InterfaceLanguageCode, LocaleLoader>

export async function loadLocaleResources(
  language: string
): Promise<Record<string, string>> {
  const locale = normalizeInterfaceLanguage(language)
  const module = await localeLoaders[locale]()
  return module.default.translation
}

async function readLocale(
  language: string,
  callback: ReadCallback
): Promise<void> {
  try {
    callback(null, await loadLocaleResources(language))
  } catch (error: unknown) {
    callback(
      error instanceof Error ? error : new Error('failed to load locale'),
      false
    )
  }
}

const localeBackend: BackendModule = {
  type: 'backend',
  init() {},
  read(language: string, _namespace: string, callback: ReadCallback) {
    void readLocale(language, callback)
  },
}

function syncDocumentLanguage() {
  if (typeof document === 'undefined') return
  document.documentElement.lang = toIntlLocale(i18n.resolvedLanguage) ?? 'en'
}

i18n.on('initialized', syncDocumentLanguage)
i18n.on('languageChanged', syncDocumentLanguage)
i18n.on('loaded', syncDocumentLanguage)

i18n
  .use(localeBackend)
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    fallbackLng: 'en',
    supportedLngs: ['en', 'zhCN', 'fr', 'ru', 'ja', 'vi', 'zhTW'],
    load: 'currentOnly',
    nsSeparator: false, // Allow literal colons in keys (e.g., URLs, labels)
    debug: import.meta.env.DEV,
    react: {
      useSuspense: true,
    },
    interpolation: {
      escapeValue: false, // not needed for react as it escapes by default
    },
    detection: {
      order: ['localStorage', 'navigator'],
      caches: ['localStorage'],
      // Browsers report `zh-CN`/`zh-TW`/`zh`; map them onto our `zhCN`/`zhTW`
      // codes (non-Chinese codes pass through for normal supportedLngs matching).
      convertDetectedLanguage,
    },
  })

export default i18n
