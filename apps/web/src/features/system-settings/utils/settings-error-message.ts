/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import i18next from 'i18next'

import {
  getServerErrorMessage,
  getServerErrorMessageKey,
} from '@/lib/server-error-message'

/** Keep a recognized public error or the server's reason before Axios's generic status. */
export function getSettingsErrorMessage(
  error: unknown,
  fallback: string
): string {
  const key = getServerErrorMessageKey(error)
  if (key) return i18next.t(key)
  return getServerErrorMessage(error) ?? fallback
}
