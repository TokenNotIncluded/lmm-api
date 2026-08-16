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
import type { SystemStatus } from '../types'

type RegistrationSetting =
  | 'register_enabled'
  | 'password_register_enabled'
  | 'oauth_register_enabled'

function readRegistrationSetting(
  status: SystemStatus | null | undefined,
  key: RegistrationSetting
): boolean | undefined {
  const direct = status?.[key]
  if (typeof direct === 'boolean') return direct

  const nested = status?.data?.[key]
  return typeof nested === 'boolean' ? nested : undefined
}

export function isRegistrationEnabled(
  status: SystemStatus | null | undefined
): boolean {
  return readRegistrationSetting(status, 'register_enabled') !== false
}

export function isPasswordRegistrationEnabled(
  status: SystemStatus | null | undefined
): boolean {
  return (
    isRegistrationEnabled(status) &&
    readRegistrationSetting(status, 'password_register_enabled') !== false
  )
}

export function isOAuthRegistrationEnabled(
  status: SystemStatus | null | undefined
): boolean {
  return (
    isRegistrationEnabled(status) &&
    readRegistrationSetting(status, 'oauth_register_enabled') !== false
  )
}

export function getDisabledOAuthRegistrationMethods(
  status: SystemStatus | null | undefined
): ReadonlySet<string> {
  const methods =
    status?.oauth_registration_disabled_methods ??
    status?.data?.oauth_registration_disabled_methods ??
    []

  return new Set(
    methods
      .map((method) => method.trim().toLowerCase())
      .filter((method) => method.length > 0)
  )
}

/**
 * Returns whether at least one configured provider can create a new account.
 * Existing sign-in providers intentionally do not use this helper: the
 * registration policy must never hide or disable login.
 */
export function hasOAuthRegistrationProvider(
  status: SystemStatus | null | undefined
): boolean {
  if (!status || !isOAuthRegistrationEnabled(status)) return false

  const disabled = getDisabledOAuthRegistrationMethods(status)
  const isAllowed = (method: string) => !disabled.has(method)

  if (status.wechat_login && isAllowed('wechat')) return true
  if (status.github_oauth && isAllowed('github')) return true
  if (status.discord_oauth && isAllowed('discord')) return true
  if (status.oidc_enabled && isAllowed('oidc')) return true
  if (status.linuxdo_oauth && isAllowed('linuxdo')) return true
  if (status.telegram_oauth && isAllowed('telegram')) return true

  return (status.custom_oauth_providers ?? []).some((provider) =>
    isAllowed(`custom:${provider.slug.trim().toLowerCase()}`)
  )
}

export function hasRegistrationMethod(
  status: SystemStatus | null | undefined
): boolean {
  return (
    isPasswordRegistrationEnabled(status) ||
    hasOAuthRegistrationProvider(status)
  )
}
