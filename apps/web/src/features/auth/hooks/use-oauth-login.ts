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
import axios from 'axios'
import { useState, useRef, useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { api, isAuthBundle } from '@/lib/api'
import { AuthCookieBlockedError } from '@/lib/auth-cookie'
import { rateLimitMessage } from '@/lib/request-rate-limit'
import { getServerErrorMessageKey } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import {
  createOAuthFlow,
  logout,
  startTelegramLogin,
  telegramLogin,
} from '../api'
import {
  EXTERNAL_BROWSER_PRESETS,
  type ExternalBrowserProvider,
  buildExternalBrowserStartRequest,
  parseBrowserAuthorizationResponse,
  validateExternalBrowserAuthorizationUrl,
} from '../lib/external-oauth'
import {
  GOOGLE_BUILT_IN_PRESET,
  buildGoogleBrowserStartRequest,
  validateGoogleBrowserAuthorizationUrl,
} from '../lib/google-oauth'
import {
  buildGitHubOAuthUrl,
  buildDiscordOAuthUrl,
  buildOIDCOAuthUrl,
  buildLinuxDOOAuthUrl,
} from '../lib/oauth'
import {
  parseTelegramMFAChallenge,
  pickTelegramAuthorization,
  type TelegramLoginFlow,
} from '../lib/telegram-login'
import type { SystemStatus, CustomOAuthProviderInfo } from '../types'
import { useAuthRedirect } from './use-auth-redirect'

const GITHUB_AUTHORIZATION_ENDPOINT = 'https://github.com/login/oauth/authorize'
const DISCORD_AUTHORIZATION_ENDPOINT = 'https://discord.com/oauth2/authorize'
const LINUX_DO_AUTHORIZATION_ENDPOINT =
  'https://connect.linux.do/oauth2/authorize'
const LOOPBACK_HOSTNAMES = new Set(['localhost', '127.0.0.1', '::1', '[::1]'])

function trustedOAuthEndpoint(value: string): URL | null {
  try {
    const url = new URL(value)
    const isSecure = url.protocol === 'https:'
    const isLocalDevelopment =
      import.meta.env.DEV &&
      typeof window !== 'undefined' &&
      url.protocol === 'http:' &&
      LOOPBACK_HOSTNAMES.has(window.location.hostname) &&
      LOOPBACK_HOSTNAMES.has(url.hostname)

    if (
      (!isSecure && !isLocalDevelopment) ||
      !url.hostname ||
      url.username ||
      url.password ||
      url.hash
    ) {
      return null
    }
    return url
  } catch {
    return null
  }
}

function navigateToTrustedOAuthEndpoint(
  value: string,
  configuredEndpoint: string
): void {
  const target = trustedOAuthEndpoint(value)
  const endpoint = trustedOAuthEndpoint(configuredEndpoint)
  if (
    !target ||
    !endpoint ||
    target.origin !== endpoint.origin ||
    target.pathname !== endpoint.pathname
  ) {
    throw new Error('Untrusted OAuth authorization endpoint')
  }

  window.location.assign(target.toString())
}

/**
 * Hook for managing OAuth login
 */
export function useOAuthLogin(
  status: SystemStatus | null,
  redirectTo?: string,
  acceptedLegal = false
) {
  const { t } = useTranslation()
  const { handleLoginSuccess, redirectTo2FA } = useAuthRedirect()
  const [isLoading, setIsLoading] = useState(false)
  const [isTelegramDialogOpen, setIsTelegramDialogOpen] = useState(false)
  const [isTelegramPending, setIsTelegramPending] = useState(false)
  const telegramController = useRef<AbortController | null>(null)
  const telegramFlow = useRef<{
    flow: TelegramLoginFlow
    accountWriteEpoch: number
    submitted: boolean
  } | null>(null)
  const [githubButtonText, setGithubButtonText] = useState('')
  const [githubButtonDisabled, setGithubButtonDisabled] = useState(false)
  const githubTimeoutRef = useRef<NodeJS.Timeout | null>(null)
  const browserStartPending = useRef(false)

  useEffect(() => () => telegramController.current?.abort(), [])

  useEffect(() => {
    setGithubButtonText(t('Continue with GitHub'))

    return () => {
      if (githubTimeoutRef.current) {
        clearTimeout(githubTimeoutRef.current)
      }
    }
  }, [t])

  const resetSession = async () => {
    const response = await logout()
    if (!response.success) {
      throw new Error(response.message || t('Failed to sign out session'))
    }
  }

  const handleGoogleLogin = async () => {
    if (browserStartPending.current) return
    const payload = buildGoogleBrowserStartRequest(status, acceptedLegal)
    if (!payload) {
      toast.error(t('Login failed'))
      return
    }

    browserStartPending.current = true
    setIsLoading(true)
    try {
      // The built-in browser bridge owns state, PKCE and the HttpOnly binding
      // cookie. Do not use the generic legacy flow or native credential API.
      const response = await api.post<unknown>(
        GOOGLE_BUILT_IN_PRESET.browser_start,
        payload,
        {
          skipAuthRefresh: true,
        }
      )
      const authorizationUrl = validateGoogleBrowserAuthorizationUrl(
        parseBrowserAuthorizationResponse(response.data),
        status?.google_client_id,
        window.location.origin,
        import.meta.env.DEV,
        status?.google_browser_callback
      )
      window.location.assign(authorizationUrl)
    } catch {
      toast.error(
        t('Failed to start {{provider}} login', { provider: 'Google' })
      )
    } finally {
      browserStartPending.current = false
      setIsLoading(false)
    }
  }

  const startExternalBrowserLogin = async (
    provider: ExternalBrowserProvider
  ) => {
    const payload = buildExternalBrowserStartRequest(
      provider,
      status,
      acceptedLegal
    )
    if (!payload) throw new Error('External browser login unavailable')
    browserStartPending.current = true
    try {
      const response = await api.post<unknown>(
        EXTERNAL_BROWSER_PRESETS[provider].browser_start,
        payload,
        { skipAuthRefresh: true }
      )
      const authorizationUrl = validateExternalBrowserAuthorizationUrl({
        provider,
        value: parseBrowserAuthorizationResponse(response.data),
        clientId: status?.[`${provider}_client_id`],
        origin: window.location.origin,
        allowLocalDevelopment: import.meta.env.DEV,
      })
      window.location.assign(authorizationUrl)
    } finally {
      browserStartPending.current = false
    }
  }

  const handleGitHubLogin = async () => {
    if (browserStartPending.current) return
    if (!status?.github_client_id) return
    if (githubButtonDisabled) return

    setIsLoading(true)
    setGithubButtonDisabled(true)
    setGithubButtonText(t('Redirecting to GitHub...'))

    if (githubTimeoutRef.current) {
      clearTimeout(githubTimeoutRef.current)
    }

    githubTimeoutRef.current = setTimeout(() => {
      setIsLoading(false)
      setGithubButtonText(
        t('Request timed out, please refresh and restart GitHub login')
      )
      setGithubButtonDisabled(true)
    }, 20000)

    try {
      if (status.external_oauth_browser === true) {
        await startExternalBrowserLogin('github')
        return
      }
      await resetSession()
      const state = await createOAuthFlow('github', 'login', acceptedLegal)

      const url = buildGitHubOAuthUrl(status.github_client_id, state)
      navigateToTrustedOAuthEndpoint(url, GITHUB_AUTHORIZATION_ENDPOINT)
    } catch {
      toast.error(t('Failed to start GitHub login'))
      if (githubTimeoutRef.current) {
        clearTimeout(githubTimeoutRef.current)
      }
      setIsLoading(false)
      setGithubButtonText(t('Continue with GitHub'))
      setGithubButtonDisabled(false)
    }
  }

  const handleDiscordLogin = async () => {
    if (browserStartPending.current) return
    if (!status?.discord_client_id) return

    setIsLoading(true)
    try {
      if (status.external_oauth_browser === true) {
        await startExternalBrowserLogin('discord')
        return
      }
      await resetSession()
      const state = await createOAuthFlow('discord', 'login', acceptedLegal)

      const url = buildDiscordOAuthUrl(status.discord_client_id, state)
      navigateToTrustedOAuthEndpoint(url, DISCORD_AUTHORIZATION_ENDPOINT)
    } catch {
      toast.error(t('Failed to start Discord login'))
    } finally {
      setIsLoading(false)
    }
  }

  const handleOIDCLogin = async () => {
    if (!status?.oidc_authorization_endpoint || !status?.oidc_client_id) return

    setIsLoading(true)
    try {
      await resetSession()
      const state = await createOAuthFlow('oidc', 'login', acceptedLegal)

      const url = buildOIDCOAuthUrl(
        status.oidc_authorization_endpoint,
        status.oidc_client_id,
        state
      )
      navigateToTrustedOAuthEndpoint(url, status.oidc_authorization_endpoint)
    } catch {
      toast.error(t('Failed to start OIDC login'))
    } finally {
      setIsLoading(false)
    }
  }

  const handleLinuxDOLogin = async () => {
    if (browserStartPending.current) return
    if (!status?.linuxdo_client_id) return

    setIsLoading(true)
    try {
      if (status.external_oauth_browser === true) {
        await startExternalBrowserLogin('linuxdo')
        return
      }
      await resetSession()
      const state = await createOAuthFlow('linuxdo', 'login', acceptedLegal)

      const url = buildLinuxDOOAuthUrl(status.linuxdo_client_id, state)
      navigateToTrustedOAuthEndpoint(url, LINUX_DO_AUTHORIZATION_ENDPOINT)
    } catch {
      toast.error(t('Failed to start LinuxDO login'))
    } finally {
      setIsLoading(false)
    }
  }

  const handleTelegramLogin = async () => {
    if (telegramController.current) return
    if (!status?.telegram_bot_name?.trim()) {
      toast.error(t('Login failed'))
      return
    }

    const controller = new AbortController()
    telegramController.current = controller
    setIsLoading(true)
    try {
      await resetSession()
      if (controller.signal.aborted) return
      const flow = await startTelegramLogin(controller.signal)
      if (
        controller.signal.aborted ||
        telegramController.current !== controller
      ) {
        return
      }
      telegramFlow.current = {
        flow,
        accountWriteEpoch: useAuthStore.getState().auth.accountWriteEpoch,
        submitted: false,
      }
      setIsTelegramDialogOpen(true)
    } catch (error: unknown) {
      if (telegramController.current === controller) {
        telegramController.current = null
        telegramFlow.current = null
      }
      if (controller.signal.aborted || axios.isCancel(error)) return
      showTelegramError(
        error,
        t('Failed to start {{provider}} login', { provider: 'Telegram' })
      )
    } finally {
      if (
        telegramController.current === controller ||
        !telegramController.current
      ) {
        setIsLoading(false)
      }
    }
  }

  const showTelegramError = (error: unknown, fallback = t('Login failed')) => {
    if (error instanceof AuthCookieBlockedError) {
      toast.error(error.message)
      return
    }
    const messageKey = getServerErrorMessageKey(error)
    const safeKey =
      messageKey === 'Session expired!'
        ? null
        : messageKey ===
            'This account is disabled. Use the appeal form below to ask an administrator to restore access.'
          ? 'This user account is disabled.'
          : messageKey
    const limited = rateLimitMessage(error)
    toast.error(
      limited ?? (safeKey ? t(safeKey) : fallback),
      limited ? { id: 'request-rate-limited' } : undefined
    )
  }

  const handleTelegramAuthorization = async (value: unknown) => {
    const controller = telegramController.current
    const pending = telegramFlow.current
    if (
      !controller ||
      controller.signal.aborted ||
      !pending ||
      pending.submitted
    ) {
      return
    }
    if (
      useAuthStore.getState().auth.accountWriteEpoch !==
      pending.accountWriteEpoch
    ) {
      handleTelegramDialogChange(false)
      return
    }
    if (pending.flow.expires_at <= Date.now() / 1000) {
      handleTelegramDialogChange(false)
      toast.error(t('Login flow expired. Please sign in again.'))
      return
    }
    const authorization = pickTelegramAuthorization(value)
    if (!authorization) {
      handleTelegramDialogChange(false)
      toast.error(t('Login failed'))
      return
    }

    pending.submitted = true
    setIsTelegramPending(true)
    const isCurrent = () =>
      !controller.signal.aborted && telegramController.current === controller
    try {
      const response = await telegramLogin(
        authorization,
        pending.flow.flow_token,
        controller.signal
      )
      if (!isCurrent()) return
      setIsTelegramDialogOpen(false)
      telegramFlow.current = null
      if (!response.success) {
        showTelegramError(response)
        return
      }

      const challenge = parseTelegramMFAChallenge(response.data)
      if (challenge) {
        if (
          useAuthStore.getState().auth.pending2FAFlowToken !==
          challenge.flow_token
        ) {
          return
        }
        redirectTo2FA(redirectTo)
        return
      }
      if (!isAuthBundle(response.data)) {
        toast.error(t('Login failed'))
        return
      }
      const bundle = response.data
      const hasCurrentBundle = () => {
        const auth = useAuthStore.getState().auth
        return (
          auth.user?.id === bundle.user.id &&
          auth.session?.sid === bundle.session.sid &&
          auth.accessToken === bundle.access_token
        )
      }
      if (!hasCurrentBundle()) return
      await handleLoginSuccess(bundle, redirectTo, true, controller.signal)
      if (isCurrent() && hasCurrentBundle()) toast.success(t('Welcome back!'))
    } catch (error: unknown) {
      if (!isCurrent() || axios.isCancel(error)) return
      setIsTelegramDialogOpen(false)
      showTelegramError(error)
    } finally {
      if (telegramController.current === controller) {
        telegramController.current = null
        telegramFlow.current = null
        setIsTelegramPending(false)
      }
    }
  }

  const handleTelegramDialogChange = (open: boolean) => {
    setIsTelegramDialogOpen(open)
    if (!open) {
      telegramController.current?.abort()
      telegramController.current = null
      telegramFlow.current = null
      setIsTelegramPending(false)
      setIsLoading(false)
    }
  }

  const handleCustomOAuthLogin = async (provider: CustomOAuthProviderInfo) => {
    if (!provider.authorization_endpoint || !provider.client_id) return

    setIsLoading(true)
    try {
      await resetSession()
      const state = await createOAuthFlow(provider.slug, 'login', acceptedLegal)

      const redirectUri = `${window.location.origin}/oauth/${provider.slug}`
      const url = new URL(provider.authorization_endpoint)
      url.searchParams.set('client_id', provider.client_id)
      url.searchParams.set('redirect_uri', redirectUri)
      url.searchParams.set('response_type', 'code')
      url.searchParams.set('state', state)
      if (provider.scopes) {
        url.searchParams.set('scope', provider.scopes)
      }

      navigateToTrustedOAuthEndpoint(
        url.toString(),
        provider.authorization_endpoint
      )
    } catch {
      toast.error(
        t('Failed to start {{provider}} login', { provider: provider.name })
      )
    } finally {
      setIsLoading(false)
    }
  }

  return {
    isLoading,
    githubButtonText,
    githubButtonDisabled,
    isTelegramDialogOpen,
    isTelegramPending,
    handleGoogleLogin,
    handleGitHubLogin,
    handleDiscordLogin,
    handleOIDCLogin,
    handleLinuxDOLogin,
    handleTelegramLogin,
    handleTelegramAuthorization,
    handleTelegramDialogChange,
    handleCustomOAuthLogin,
  }
}
