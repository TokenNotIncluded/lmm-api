/*
Copyright (C) 2026 LIghtJUNction

SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { z } from 'zod'

import type { SystemStatus } from '../types'

export type ExternalBrowserProvider = 'github' | 'discord' | 'linuxdo'

export const EXTERNAL_BROWSER_PRESETS = Object.freeze({
  github: {
    authorization_endpoint: 'https://github.com/login/oauth/authorize',
    browser_start: '/api/oauth/github/start',
    callbacks: ['/oauth/github', '/api/oauth/github'],
    scope: 'read:user',
    pkce: true,
  },
  discord: {
    authorization_endpoint: 'https://discord.com/oauth2/authorize',
    browser_start: '/api/oauth/discord/start',
    callbacks: ['/oauth/discord'],
    scope: 'identify',
    pkce: true,
  },
  linuxdo: {
    authorization_endpoint: 'https://connect.linux.do/oauth2/authorize',
    browser_start: '/api/oauth/linuxdo/start',
    callbacks: ['/api/oauth/linuxdo', '/oauth/linuxdo'],
    scope: 'user',
    // LinuxDO's confidential authorization-code contract does not use PKCE.
    pkce: false,
  },
})

export function buildExternalBrowserStartRequest(
  provider: ExternalBrowserProvider,
  status: SystemStatus | null | undefined,
  acceptedLegal: boolean
): { accepted_legal: boolean } | null {
  const clientId = status?.[`${provider}_client_id`]
  if (
    status?.external_oauth_browser !== true ||
    status[`${provider}_oauth`] !== true ||
    typeof clientId !== 'string' ||
    !clientId.trim() ||
    ((status.user_agreement_enabled === true ||
      status.privacy_policy_enabled === true) &&
      !acceptedLegal)
  ) {
    return null
  }
  return { accepted_legal: acceptedLegal }
}

export function validateExternalBrowserAuthorizationUrl({
  provider,
  value,
  clientId,
  origin,
  allowLocalDevelopment = false,
}: {
  provider: ExternalBrowserProvider
  value: unknown
  clientId: unknown
  origin: string
  allowLocalDevelopment?: boolean
}): string {
  const reject = (): never => {
    throw new Error('Untrusted external browser authorization URL')
  }
  if (
    typeof value !== 'string' ||
    typeof clientId !== 'string' ||
    !clientId.trim()
  ) {
    return reject()
  }
  try {
    const preset = EXTERNAL_BROWSER_PRESETS[provider]
    const site = new URL(origin)
    const target = new URL(value)
    const endpoint = new URL(preset.authorization_endpoint)
    const callback = new URL(target.searchParams.get('redirect_uri') || '')
    const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(site.hostname)
    const hasPKCE = preset.pkce
      ? Boolean(target.searchParams.get('code_challenge')?.trim()) &&
        target.searchParams.get('code_challenge_method') === 'S256'
      : !target.searchParams.has('code_challenge') &&
        !target.searchParams.has('code_challenge_method')
    if (
      (site.protocol !== 'https:' &&
        !(allowLocalDevelopment && loopback && site.protocol === 'http:')) ||
      site.username ||
      site.password ||
      target.origin !== endpoint.origin ||
      target.pathname !== endpoint.pathname ||
      target.username ||
      target.password ||
      target.hash ||
      target.searchParams.get('client_id') !== clientId ||
      target.searchParams.get('response_type') !== 'code' ||
      target.searchParams.get('scope')?.trim() !== preset.scope ||
      !target.searchParams.get('state')?.trim() ||
      !hasPKCE ||
      callback.origin !== site.origin ||
      !preset.callbacks.includes(callback.pathname) ||
      callback.username ||
      callback.password ||
      callback.search ||
      callback.hash
    ) {
      return reject()
    }
    for (const key of target.searchParams.keys()) {
      if (target.searchParams.getAll(key).length !== 1) return reject()
    }
    // Keep the server's state, callback and proof parameters intact.
    return target.toString()
  } catch {
    return reject()
  }
}

const optionalSearchText = z.string().optional().catch(undefined)

export const oauthCallbackSearchSchema = z.object({
  code: optionalSearchText,
  state: optionalSearchText,
  error: optionalSearchText,
  error_description: optionalSearchText,
  iss: optionalSearchText,
  redirect: optionalSearchText,
  telegram_bind: optionalSearchText,
  flow_token: optionalSearchText,
  error_code: optionalSearchText,
  // TanStack's default search parser converts the raw query value 1 to a number.
  external_mfa: z
    .union([z.literal('1'), z.literal(1)])
    .transform((): '1' => '1')
    .optional()
    .catch(undefined),
})

export function isExternalMFAContinuation(
  provider: string,
  search: z.infer<typeof oauthCallbackSearchSchema>
): boolean {
  return (
    ['google', 'github', 'discord', 'linuxdo'].includes(provider) &&
    search.external_mfa === '1' &&
    !search.code &&
    !search.state &&
    !search.error
  )
}

const browserAuthorizationResponseSchema = z.object({
  success: z.literal(true),
  data: z.object({ authorization_url: z.string().min(1) }),
})

export function parseBrowserAuthorizationResponse(value: unknown): string {
  return browserAuthorizationResponseSchema.parse(value).data.authorization_url
}
