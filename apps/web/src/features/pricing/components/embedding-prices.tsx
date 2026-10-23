/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useTranslation } from 'react-i18next'

import { GroupBadge } from '@/components/group-badge'

import type { PricingModel } from '../types'
import { formatCacheWriteRate } from './cache-write-prices'

export function EmbeddingPrices({ model }: { model: PricingModel }) {
  const { t } = useTranslation()
  const groups = Object.entries(model.group_embedding_prices ?? {}).filter(
    ([group]) => model.enable_groups.includes(group)
  )
  if (groups.length === 0) return null
  return (
    <section
      className='space-y-3'
      aria-label={t('Multimodal embedding prices')}
    >
      <h3 className='text-sm font-semibold'>
        {t('Multimodal embedding prices')}
      </h3>
      <p className='text-muted-foreground text-xs leading-5'>
        {t(
          'Text and image or video input are measured separately. Rates are USD per million tokens and include the selected group and any verified account discount.'
        )}
      </p>
      {groups.map(([group, price]) => (
        <div key={group} className='space-y-2 border p-3'>
          <div className='flex flex-wrap items-center gap-2 text-xs'>
            <GroupBadge group={group} size='sm' />
            <span>Ali</span>
            {!price.available && (
              <span className='text-muted-foreground'>
                {t('Embedding is not enabled for this route')}
              </span>
            )}
          </div>
          <dl className='grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-2 text-xs'>
            <dt>{t('Embedding text input tokens')}</dt>
            <dd className='text-right font-mono break-all tabular-nums'>
              {formatCacheWriteRate(price.text_usd_per_million)}
            </dd>
            <dt>{t('Embedding image and video input tokens')}</dt>
            <dd className='text-right font-mono break-all tabular-nums'>
              {formatCacheWriteRate(price.image_usd_per_million)}
            </dd>
          </dl>
        </div>
      ))}
    </section>
  )
}
