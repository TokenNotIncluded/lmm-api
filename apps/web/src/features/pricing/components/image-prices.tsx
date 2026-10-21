/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useTranslation } from 'react-i18next'

import { GroupBadge } from '@/components/group-badge'

import {
  formatImageVariantCredits,
  formatImageVariantUSD,
} from '../lib/image-price'
import type { PricingModel } from '../types'

export function ImagePrices({ model }: { model: PricingModel }) {
  const { t } = useTranslation()
  if (!model.image_variants?.length) return null
  return (
    <section className='space-y-3' aria-label={t('Image prices')}>
      <h3 className='text-sm font-semibold'>{t('Image prices')}</h3>
      <p className='text-muted-foreground text-xs leading-5'>
        {t(
          'Prices include the selected group factors. Select a group to compare its rates.'
        )}
      </p>
      {model.image_variants.map((variant, index) => (
        <div
          key={`${variant.group}:${variant.endpoint}:${variant.size}:${variant.quality}:${index}`}
          className='space-y-2 border-b pb-3 last:border-b-0 last:pb-0'
        >
          <div className='flex flex-wrap items-center gap-2'>
            <GroupBadge group={variant.group} size='sm' />
            <code className='text-muted-foreground text-xs'>
              /v1/images/{variant.endpoint}
            </code>
          </div>
          <dl className='grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] gap-x-4 gap-y-1 text-xs'>
            <dt className='text-muted-foreground'>{t('Size:')}</dt>
            <dd className='break-words'>{variant.size}</dd>
            <dt className='text-muted-foreground'>{t('Quality (optional)')}</dt>
            <dd className='break-words'>{variant.quality}</dd>
            <dt className='text-muted-foreground'>
              {t(
                variant.rate_basis === 'effective_user_rate'
                  ? 'Current account price'
                  : variant.rate_basis === 'base_group_rate'
                    ? 'Group base price'
                    : 'Price'
              )}
            </dt>
            <dd className='font-mono break-all tabular-nums'>
              {formatImageVariantUSD(variant)} / {t('Image')}
            </dd>
            <dt className='text-muted-foreground'>Credits</dt>
            <dd className='font-mono break-all tabular-nums'>
              {formatImageVariantCredits(variant)} / {t('Image')}
            </dd>
            <dt className='text-muted-foreground'>{t('Billed image count')}</dt>
            <dd>
              {t(
                variant.count_basis === 'requested'
                  ? 'Requested images'
                  : 'Returned images'
              )}
            </dd>
            <dt className='text-muted-foreground'>{t('Maximum images')}</dt>
            <dd>{variant.max_n}</dd>
          </dl>
          <p className='text-muted-foreground text-xs'>
            {t('Charges round up to whole Credits per request.')}
          </p>
        </div>
      ))}
    </section>
  )
}
