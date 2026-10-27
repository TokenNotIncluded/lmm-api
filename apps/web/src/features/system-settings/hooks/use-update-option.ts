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
/*
Copyright (C) 2026 LIghtJUNction
*/
import { useMutation, useQueryClient } from '@tanstack/react-query'
import i18next from 'i18next'
import { toast } from 'sonner'

import { capabilityPhase } from '@/lib/runtime-capabilities'
import { getServerErrorToastId } from '@/lib/server-error-message'

import {
  updateAssistantSystemOptions,
  updateSystemOption,
  updateSystemOptions,
} from '../api'
import {
  ASSISTANT_TOOL_POLICY_CONFLICT_MESSAGE,
  isAssistantToolPolicyConflict,
} from '../content/assistant-tool-policy-save'
import { updatePublicCreditUnitOption } from '../general/public-credit-unit'
import type {
  UpdateAssistantOptionsRequest,
  UpdateOptionRequest,
  UpdateOptionResponse,
  SystemOptionsResponse,
} from '../types'
import { showOptionUpdateToast } from '../utils/option-update-toast'
import { getSettingsErrorMessage } from '../utils/settings-error-message'
import { writableOptionKey } from './option-writability'

// Configuration keys that require status refresh
const STATUS_RELATED_KEYS = new Set([
  'HeaderNavModules',
  'SidebarModulesAdmin',
  'Notice',
  'LogConsumeEnabled',
  'QuotaPerUnit',
  'PublicCreditsPerUSD',
  'USDExchangeRate',
  'DisplayInCurrencyEnabled',
  'DisplayTokenStatEnabled',
  'general_setting.quota_display_type',
  'general_setting.custom_currency_symbol',
  'general_setting.custom_currency_exchange_rate',
  'oidc.display_name',
  'OAuthRegisterEnabled',
  'ModerationEnabled',
  'ModerationPolicyScope',
  'ModerationSafetyIdentifierEnabled',
  'ModerationGroup',
  'ModerationModel',
  'ModerationGroupPolicies',
  'AssistantModerationEnabled',
  'AssistantModerationGroup',
  'AssistantModerationModel',
  'AssistantEnabled',
  'AssistantGroup',
  'AssistantModel',
  'AssistantReasoningEffort',
  'AssistantStreamEnabled',
  'AssistantTemperature',
  'AssistantMaxTokens',
  'AssistantNewUserGiftMaxCredits',
  'AssistantAgentLoopEnabled',
  'AssistantMaxSteps',
  'AssistantTimeoutSeconds',
  'AssistantCacheEnabled',
  'AssistantCacheTTLMinutes',
  'AssistantPersona',
  'AssistantSystemPrompt',
  'AssistantSearchProvider',
  'AssistantSearchURL',
  'AssistantSearchAPIKey',
  'AssistantSearchMCPTool',
  'AssistantToolPolicy',
  'AssistantSkills',
  'AssistantSkillFiles',
  'AssistantRetentionEnabled',
  'AssistantActiveRetentionDays',
  'AssistantArchivedRetentionDays',
  'AssistantSecurityRetentionDays',
  'AssistantRetentionIntervalHours',
])

async function invalidateOptionQueries(
  queryClient: ReturnType<typeof useQueryClient>,
  keys: Iterable<string>
) {
  const changedKeys = [...keys]
  const invalidate = (queryKey: string[]) =>
    queryClient.invalidateQueries({ queryKey }, { throwOnError: true })
  const refreshes = [invalidate(['system-options'])]
  if (changedKeys.some((key) => key.startsWith('Assistant'))) {
    refreshes.push(invalidate(['assistant-status']))
  }
  if (
    changedKeys.some(
      (key) =>
        key.startsWith('Moderation') || key.startsWith('AssistantModeration')
    )
  ) {
    refreshes.push(
      invalidate(['security-policy']),
      invalidate(['admin-security-policy']),
      invalidate(['moderation-routing-models']),
      invalidate(['admin-moderation-reviews']),
      invalidate(['admin-moderation-stats'])
    )
  }
  if (changedKeys.includes('AIDirectoryLinks')) {
    refreshes.push(invalidate(['ai-directory']))
  }
  if (changedKeys.includes('About')) {
    refreshes.push(invalidate(['about-content']))
  }
  if (changedKeys.includes('RSSFeeds')) {
    refreshes.push(invalidate(['rss']))
  }
  if (changedKeys.includes('AssistantNewUserGiftMaxCredits')) {
    refreshes.push(invalidate(['assistant-new-user-gift']))
  }
  if (changedKeys.includes('AssistantPreConversationPresets')) {
    refreshes.push(invalidate(['assistant-pre-conversation-presets']))
  }
  if (changedKeys.some((key) => STATUS_RELATED_KEYS.has(key))) {
    refreshes.push(invalidate(['status']))
    try {
      window.localStorage.removeItem('status')
    } catch {
      /* empty */
    }
  }
  await Promise.all(refreshes)
}

let nextSaveToast = 0

function acceptSettingsSave(response: UpdateOptionResponse) {
  if (!response.success) {
    throw Object.assign(
      new Error(response.message || i18next.t('Failed to update setting')),
      {
        response: { data: response },
      }
    )
  }
  return response
}

function refreshAcknowledgedOptions(
  queryClient: ReturnType<typeof useQueryClient>,
  response: UpdateOptionResponse,
  keys: Iterable<string>
) {
  const id = `settings-save-${++nextSaveToast}`
  showOptionUpdateToast(response, i18next.t('Setting updated successfully'), {
    id,
  })
  // Refreshing has its own outcome. Never reject an acknowledged write or
  // make a caller repeat it because a subsequent read is unavailable.
  void invalidateOptionQueries(queryClient, keys).catch((error: unknown) => {
    toast.warning(
      i18next.t('Settings saved, but refreshing failed: {{reason}}', {
        reason: getSettingsErrorMessage(
          error,
          i18next.t('Failed to load settings')
        ),
      }),
      { id, description: response.warnings?.join('\n') }
    )
  })
}

function reportSaveFailure(error: unknown) {
  toast.error(
    getSettingsErrorMessage(error, i18next.t('Failed to update setting')),
    {
      id: getServerErrorToastId(error) ?? undefined,
    }
  )
}

function requireCurrentOptions(queryClient: ReturnType<typeof useQueryClient>) {
  const state = queryClient.getQueryState<SystemOptionsResponse>([
    'system-options',
  ])
  if (
    !state?.data ||
    state.isInvalidated ||
    capabilityPhase(state) !== 'fresh'
  ) {
    throw new Error(
      i18next.t(
        'Capability information is not current. Refresh before making changes.'
      )
    )
  }
  return state.data
}

export function useUpdateOption() {
  const queryClient = useQueryClient()
  return useMutation({
    retry: false,
    mutationFn: async (request: UpdateOptionRequest) => {
      const options = requireCurrentOptions(queryClient)
      const key = writableOptionKey(options?.capabilities, request.key)
      if (!key) throw new Error(i18next.t('Read only'))
      const allowedRequest = { ...request, key }
      return acceptSettingsSave(
        request.key === 'PublicCreditsPerUSD'
          ? await updatePublicCreditUnitOption(allowedRequest)
          : await updateSystemOption(allowedRequest, { silent: true })
      )
    },
    onSuccess: (data, variables) =>
      refreshAcknowledgedOptions(queryClient, data, [variables.key]),
    onError: reportSaveFailure,
  })
}

export function useUpdateOptions() {
  const queryClient = useQueryClient()
  return useMutation({
    retry: false,
    mutationFn: async (values: Record<string, string>) => {
      const options = requireCurrentOptions(queryClient)
      const allowedValues: Record<string, string> = {}
      for (const [key, value] of Object.entries(values)) {
        const allowedKey = writableOptionKey(options?.capabilities, key)
        if (!allowedKey) throw new Error(i18next.t('Read only'))
        if (
          allowedValues[allowedKey] !== undefined &&
          allowedValues[allowedKey] !== value
        ) {
          throw new Error(i18next.t('Read only'))
        }
        allowedValues[allowedKey] = value
      }
      return acceptSettingsSave(
        await updateSystemOptions(allowedValues, { silent: true })
      )
    },
    onSuccess: (data, variables) =>
      refreshAcknowledgedOptions(queryClient, data, Object.keys(variables)),
    onError: reportSaveFailure,
  })
}

export function useUpdateAssistantOptions() {
  const queryClient = useQueryClient()
  return useMutation({
    retry: false,
    mutationFn: async (request: UpdateAssistantOptionsRequest) => {
      const options = requireCurrentOptions(queryClient)
      if (
        Object.keys(request.values).some(
          (key) => !writableOptionKey(options?.capabilities, key)
        )
      ) {
        throw new Error(i18next.t('Read only'))
      }
      return acceptSettingsSave(
        await updateAssistantSystemOptions(request, { silent: true })
      )
    },
    onSuccess: (data, request) =>
      refreshAcknowledgedOptions(
        queryClient,
        data,
        Object.keys(request.values)
      ),
    onError: (error: unknown) => {
      if (isAssistantToolPolicyConflict(error)) {
        toast.error(i18next.t(ASSISTANT_TOOL_POLICY_CONFLICT_MESSAGE))
      } else {
        reportSaveFailure(error)
      }
    },
  })
}
