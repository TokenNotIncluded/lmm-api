/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
export const searchProviders = [
  'open_ai_responses_v1',
  'gemini_grounding_v1',
  'azure_responses_v1',
  'xai_responses_v1',
  'dashscope_responses_v1',
  'dashscope_anthropic_v1',
  'anthropic_messages_v1',
  'zhipu_chat_v1',
] as const

export type SearchProvider = (typeof searchProviders)[number]
export type SearchBillingUnit = 'call' | 'query' | 'grounded_request'

export const searchUnitLabels = {
  call: 'Search calls',
  query: 'Search queries',
  grounded_request: 'Requests with search',
} as const

export const searchProviderLabels: Record<SearchProvider, string> = {
  open_ai_responses_v1: 'OpenAI',
  gemini_grounding_v1: 'Google Search',
  azure_responses_v1: 'Azure Bing Search',
  xai_responses_v1: 'xAI',
  dashscope_responses_v1: 'DashScope',
  dashscope_anthropic_v1: 'DashScope',
  anthropic_messages_v1: 'Anthropic',
  zhipu_chat_v1: 'Zhipu',
}

export function searchUnitMatchesProvider(
  provider: SearchProvider,
  unit: SearchBillingUnit
): boolean {
  if (provider === 'gemini_grounding_v1') {
    return unit === 'query' || unit === 'grounded_request'
  }
  return unit === (provider === 'zhipu_chat_v1' ? 'grounded_request' : 'call')
}
