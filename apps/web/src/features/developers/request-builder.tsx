/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { useQuery } from '@tanstack/react-query'
import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { getPricing, pricingQueryKey } from '@/features/pricing/api'
import { useCopyToClipboard } from '@/hooks/use-copy-to-clipboard'
import { useAuthStore } from '@/stores/auth-store'

import { LMM_ISSUER } from './integration-prompts'
import { buildRequestBody, buildRequestSnippet } from './request-snippet'

import './request-builder.css'

type Protocol = 'chat' | 'messages' | 'gemini'
type Language = 'curl' | 'javascript' | 'python'

const PROTOCOLS: readonly {
  id: Protocol
  label: string
  path: string
  shape: 'openai' | 'anthropic' | 'gemini'
}[] = [
  {
    id: 'chat',
    label: 'Chat Completions',
    path: '/v1/chat/completions',
    shape: 'openai',
  },
  {
    id: 'messages',
    label: 'Claude Messages',
    path: '/v1/messages',
    shape: 'anthropic',
  },
  {
    id: 'gemini',
    label: 'Gemini',
    path: '/v1beta/models/{model}:generateContent',
    shape: 'gemini',
  },
]

const LANGUAGES: readonly { id: Language; label: string }[] = [
  { id: 'curl', label: 'cURL' },
  { id: 'javascript', label: 'JavaScript' },
  { id: 'python', label: 'Python' },
]

const MODEL_OPTIONS: Readonly<
  Record<
    Protocol,
    {
      endpoint: string
    }
  >
> = {
  chat: {
    endpoint: 'openai',
  },
  messages: {
    endpoint: 'anthropic',
  },
  gemini: {
    endpoint: 'gemini',
  },
}

function sortModels(models: string[]): string[] {
  return models.sort((left, right) => {
    return left.localeCompare(right, undefined, {
      numeric: true,
      sensitivity: 'base',
    })
  })
}

/**
 * Build a real request against api.lmm.best without sending the prompt. Model
 * choices come from the live public pricing catalogue, filtered by endpoint
 * compatibility, so the examples do not age into hard-coded legacy models.
 */
export function RequestBuilder() {
  const { t } = useTranslation()
  const { user, session } = useAuthStore((state) => state.auth)
  const { copiedText, copyToClipboard } = useCopyToClipboard({ notify: false })
  const [protocol, setProtocol] = useState<Protocol>('chat')
  const [language, setLanguage] = useState<Language>('curl')
  const [model, setModel] = useState('')
  const [prompt, setPrompt] = useState('Explain LMM in one sentence.')

  const pricingQuery = useQuery({
    queryKey: pricingQueryKey(user?.id, session?.sid),
    queryFn: ({ signal }) =>
      getPricing(signal, { userId: user?.id, sessionId: session?.sid }),
    staleTime: 5 * 60_000,
    retry: false,
  })

  const modelOptions = useMemo(() => {
    if (pricingQuery.isError) return []
    const config = MODEL_OPTIONS[protocol]
    const liveModels =
      pricingQuery.data?.data
        .filter(
          (item) =>
            item.runtime_state?.status !== 'unavailable' &&
            item.supported_endpoint_types?.includes(config.endpoint)
        )
        .map((item) => item.model_name.trim())
        .filter(Boolean) ?? []

    const uniqueModels = [...new Set(liveModels)]
    return sortModels(uniqueModels)
  }, [pricingQuery.data?.data, pricingQuery.isError, protocol])

  useEffect(() => {
    if (!model && modelOptions.length > 0) {
      setModel(modelOptions[0] ?? '')
    }
  }, [model, modelOptions])

  const active = PROTOCOLS.find((item) => item.id === protocol) ?? PROTOCOLS[0]
  const url = `${LMM_ISSUER}${active.path.replace('{model}', encodeURIComponent(model || 'MODEL'))}`
  const body = useMemo(
    () => buildRequestBody(active.shape, model || 'MODEL', prompt || 'Hello'),
    [active.shape, model, prompt]
  )
  const snippet = useMemo(
    () => buildRequestSnippet(language, url, body),
    [language, url, body]
  )
  const copied = copiedText === snippet

  return (
    <div className='dev-builder mt-6'>
      <div className='dev-builder-row'>
        <span className='dev-builder-label'>{t('Protocol')}</span>
        <div
          role='group'
          aria-label={t('Protocol')}
          className='dev-builder-tabs'
        >
          {PROTOCOLS.map((item) => (
            <button
              key={item.id}
              type='button'
              aria-pressed={protocol === item.id}
              onClick={() => {
                setProtocol(item.id)
                setModel('')
              }}
              className='dev-builder-tab'
            >
              {item.label}
            </button>
          ))}
        </div>
      </div>
      <div className='dev-builder-fields'>
        <div className='dev-builder-field'>
          <label className='dev-builder-label' htmlFor='dev-builder-model'>
            {t('Model')}
          </label>
          <input
            id='dev-builder-model'
            list='dev-builder-model-options'
            value={model}
            placeholder='MODEL'
            onChange={(event) => setModel(event.target.value)}
            className='dev-builder-input'
          />
          <datalist id='dev-builder-model-options'>
            {modelOptions.map((item) => (
              <option key={item} value={item}>
                {item}
              </option>
            ))}
          </datalist>
        </div>
        <div className='dev-builder-field'>
          <label className='dev-builder-label' htmlFor='dev-builder-prompt'>
            {t('Prompt')}
          </label>
          <input
            id='dev-builder-prompt'
            value={prompt}
            maxLength={200}
            onChange={(event) => setPrompt(event.target.value)}
            className='dev-builder-input dev-builder-prompt'
          />
        </div>
      </div>
      <div className='dev-builder-row'>
        <span className='dev-builder-label'>{t('Language')}</span>
        <div
          role='group'
          aria-label={t('Language')}
          className='dev-builder-tabs'
        >
          {LANGUAGES.map((item) => (
            <button
              key={item.id}
              type='button'
              aria-pressed={language === item.id}
              onClick={() => setLanguage(item.id)}
              className='dev-builder-tab'
            >
              {item.label}
            </button>
          ))}
        </div>
      </div>
      <div className='dev-builder-output'>
        <div className='dev-builder-output-head'>
          <code>{url}</code>
          <Button
            type='button'
            variant='outline'
            size='sm'
            aria-label={`${t('Copy')}: ${active.label} · ${language}`}
            onClick={() => void copyToClipboard(snippet)}
            className='min-h-11 shrink-0'
          >
            <span aria-live='polite'>{copied ? t('Copied') : t('Copy')}</span>
          </Button>
        </div>
        <pre tabIndex={0} aria-label={`${active.label} · ${language}`}>
          <code>{snippet}</code>
        </pre>
      </div>
      <p className='text-muted-foreground text-xs leading-relaxed'>
        {t(
          'Nothing is sent. Copy the request into your own client and use your own key.'
        )}
      </p>
    </div>
  )
}
