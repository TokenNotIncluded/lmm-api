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
import { Braces, Check, Gauge, ShieldCheck } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { getStatus } from '@/lib/api'

const REQUEST_ENDPOINT = '/v1/responses'
const REQUEST_MODEL_ROTATION_MS = 4200

export function AuthArtPanel() {
  const { t } = useTranslation()
  const [requestModels, setRequestModels] = useState<string[]>([])
  const [requestModelIndex, setRequestModelIndex] = useState(0)

  useEffect(() => {
    let active = true

    void getStatus()
      .then((status) => {
        const rawModelIDs = status?.preview_model_ids
        if (!active || !Array.isArray(rawModelIDs)) return

        const modelIDs = [...new Set(rawModelIDs)]
          .filter(
            (modelID): modelID is string =>
              typeof modelID === 'string' && modelID.trim().length > 0
          )
          .map((modelID) => modelID.trim())

        setRequestModels(modelIDs)
        setRequestModelIndex(0)
      })
      .catch(() => undefined)

    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    if (requestModels.length < 2) return

    const rotation = window.setInterval(() => {
      setRequestModelIndex((current) => (current + 1) % requestModels.length)
    }, REQUEST_MODEL_ROTATION_MS)

    return () => window.clearInterval(rotation)
  }, [requestModels.length])

  const requestModel = requestModels[requestModelIndex] ?? '—'
  const requestLines = [
    ['POST', REQUEST_ENDPOINT],
    ['model', requestModel],
    ['stream', 'true'],
  ] as const

  const capabilities = [
    {
      icon: ShieldCheck,
      title: t('Protected access'),
      detail: t('Sessions, API keys, and account controls in one place.'),
    },
    {
      icon: Gauge,
      title: t('Visible usage'),
      detail: t('Track model calls, latency, and spend without guesswork.'),
    },
  ]

  return (
    <aside className='bg-card text-card-foreground flex h-full flex-col overflow-hidden rounded-[1.75rem] border p-8 xl:p-10'>
      <div className='flex items-center gap-4 text-xs font-semibold tracking-[0.14em] uppercase'>
        <span className='text-muted-foreground'>{t('LMM API Console')}</span>
      </div>

      <div className='my-auto max-w-2xl py-10'>
        <p className='text-muted-foreground mb-4 flex items-center gap-2 text-sm font-medium'>
          <Braces className='size-4' aria-hidden='true' />
          {t('A clear route from key to response')}
        </p>
        <h2 className='max-w-xl font-serif text-4xl leading-[1.04] tracking-[-0.04em] text-balance xl:text-5xl'>
          {t('One endpoint. Clear controls. No mystery.')}
        </h2>
        <p className='text-muted-foreground mt-5 max-w-lg text-base leading-7'>
          {t(
            'Choose a model, send a compatible request, and see exactly how access and usage are managed.'
          )}
        </p>

        <div
          className='bg-background/65 relative mt-9 overflow-hidden rounded-2xl border'
          data-live-request-preview
        >
          <span
            aria-hidden='true'
            className='auth-art-request-sweep bg-primary/60 pointer-events-none absolute inset-x-0 top-0 h-px'
          />
          <div className='border-b px-5 py-3 text-xs font-semibold tracking-[0.12em] uppercase'>
            {t('Request preview')}
          </div>
          <dl className='divide-y font-mono text-sm'>
            {requestLines.map(([label, value]) => (
              <div
                className='grid grid-cols-[5.5rem_1fr] gap-4 px-5 py-3.5'
                key={`${label}-${value}`}
              >
                <dt className='text-muted-foreground'>{label}</dt>
                <dd
                  className='auth-art-request-value truncate'
                  data-request-endpoint={label === 'POST' ? value : undefined}
                  data-request-model={label === 'model' ? value : undefined}
                >
                  {value}
                </dd>
              </div>
            ))}
          </dl>
          <div className='bg-muted/40 flex items-center justify-between gap-4 border-t px-5 py-3.5 text-sm'>
            <span className='text-muted-foreground'>{t('Response')}</span>
            <span className='text-success flex items-center gap-2 font-medium'>
              <span
                aria-hidden='true'
                className='auth-art-request-pulse bg-success size-1.5 rounded-full'
              />
              <Check className='size-4' aria-hidden='true' />
              200 · {t('stream ready')}
            </span>
          </div>
        </div>

        <div className='mt-4 grid gap-4 sm:grid-cols-2'>
          {capabilities.map(({ icon: Icon, title, detail }) => (
            <section className='rounded-2xl border p-4' key={title}>
              <Icon
                className='text-muted-foreground size-5'
                aria-hidden='true'
              />
              <h3 className='mt-4 text-sm font-semibold'>{title}</h3>
              <p className='text-muted-foreground mt-1.5 text-sm leading-6'>
                {detail}
              </p>
            </section>
          ))}
        </div>
      </div>

      <p className='text-muted-foreground border-t pt-5 text-xs leading-5'>
        {t('Open-source infrastructure for accountable model access.')}
      </p>
    </aside>
  )
}
