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
import type { ToolUIPart } from 'ai'
import { useTranslation } from 'react-i18next'

import {
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
} from '@/components/ai-elements/tool'

import type { AssistantToolTrace } from './api'

const TOOL_TITLE_KEYS: Record<string, string> = {
  navigate_to_page: 'Navigate within console',
  get_user_overview: 'Inspect user account',
  get_user_usage_summary: 'Analyze user usage',
  prepare_user_action: 'Prepare user action',
  get_service_facts: 'Read service connection facts',
  get_usage_summary: 'Analyze usage',
  get_available_models: 'Read available models',
  get_model_pricing: 'Read model pricing',
  get_account_access: 'Read account access',
  get_setup_guide: 'Read setup guide',
  calculate_cost: 'Calculate cost',
}

const TOOL_SUMMARY_KEYS: Record<string, string> = {
  navigate_to_page: 'Navigation prepared',
  get_user_overview: 'Account overview loaded',
  get_user_usage_summary: 'Usage summary loaded',
  prepare_user_action: 'User action prepared',
  get_service_facts: 'Service facts loaded',
  get_usage_summary: 'Usage summary loaded',
  get_available_models: 'Available models loaded',
  get_model_pricing: 'Model pricing loaded',
  get_account_access: 'Account access loaded',
  get_setup_guide: 'Setup guide loaded',
  calculate_cost: 'Cost estimate calculated',
  set_conversation_title: 'Conversation title updated',
}

function toolTitle(name: string): string {
  return TOOL_TITLE_KEYS[name] ?? name.replaceAll('_', ' ')
}

export function AssistantToolCalls(props: { traces: AssistantToolTrace[] }) {
  const { t } = useTranslation()
  if (props.traces.length === 0) return null

  return (
    <div className='w-full space-y-1' data-testid='assistant-tool-calls'>
      {props.traces.map((trace, index) => {
        const isError = trace.status === 'output-error'
        const isApproval = trace.status === 'approval-requested'
        const statusText = isError
          ? t('Tool failed')
          : isApproval
            ? t('Waiting for confirmation')
            : t('Tool completed')
        const summary =
          isError || isApproval
            ? statusText
            : t(TOOL_SUMMARY_KEYS[trace.name] ?? 'Tool completed')
        return (
          <Tool
            key={`${trace.name}-${index}`}
            defaultOpen
            data-testid={`assistant-tool-${index}`}
          >
            <ToolHeader
              title={t(toolTitle(trace.name))}
              type={`tool-${trace.name}` as ToolUIPart['type']}
              state={trace.status}
              summary={summary}
            />
            <ToolContent>
              <ToolInput input={trace.input ?? {}} />
              <ToolOutput
                output={isError ? undefined : summary}
                errorText={isError ? statusText : undefined}
              />
            </ToolContent>
          </Tool>
        )
      })}
    </div>
  )
}
