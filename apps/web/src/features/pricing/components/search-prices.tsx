/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useTranslation } from 'react-i18next'

import { GroupBadge } from '@/components/group-badge'
import { searchProviderLabels, searchQuantityLabel } from '@/lib/search-metering'

import type { PricingModel } from '../types'
import { formatCacheWriteRate } from './cache-write-prices'

export function SearchPrices({ model }: { model: PricingModel }) {
  const { t } = useTranslation()
  const groups = Object.entries(model.group_search_prices ?? {}).filter(
    ([group]) => model.enable_groups.includes(group)
  )
  if (groups.length === 0) return null
  return (
    <section className='space-y-3' aria-label={t('Search tool prices')}>
      <h3 className='text-sm font-semibold'>{t('Search tool prices')}</h3>
      <p className='text-muted-foreground text-xs leading-5'>
        {t(
          'Search rates are per measured unit and include the selected group and any verified account discount. Search and token charges are rounded together.'
        )}
      </p>
      {groups.map(([group, price]) => (
        <div key={group} className='space-y-2 border p-3'>
          <div className='flex flex-wrap items-center gap-2 text-xs'>
            <GroupBadge group={group} size='sm' />
            <span>{searchProviderLabels[price.provider]}</span>
            {!price.available && (
              <span className='text-muted-foreground'>
                {t('Search is not enabled for this route')}
              </span>
            )}
          </div>
          <dl className='grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-2 text-xs'>
            <dt>{t(searchQuantityLabel(price.provider, price.unit))}</dt>
            <dd className='text-right font-mono break-all tabular-nums'>
              {formatCacheWriteRate(price.usd_per_unit)}
            </dd>
            {price.x_posts_fetched_usd_per_unit && (
              <>
                <dt>{t('X posts fetched')}</dt>
                <dd className='text-right font-mono break-all tabular-nums'>
                  {formatCacheWriteRate(price.x_posts_fetched_usd_per_unit)}
                </dd>
              </>
            )}
            {price.x_users_fetched_usd_per_unit && (
              <>
                <dt>{t('X users fetched')}</dt>
                <dd className='text-right font-mono break-all tabular-nums'>
                  {formatCacheWriteRate(price.x_users_fetched_usd_per_unit)}
                </dd>
              </>
            )}
          </dl>
        </div>
      ))}
    </section>
  )
}
