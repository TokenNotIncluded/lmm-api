/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useTranslation } from 'react-i18next'

import { GroupBadge } from '@/components/group-badge'

import type { NativeCacheWrite12hTier } from '../catalog-contract'
import type { PricingModel } from '../types'

/** Display the exact rate; never round a small positive price to zero. */
export function formatCacheWriteRate(
  rate: NativeCacheWrite12hTier['input']
): string {
  const numerator = BigInt(rate.numerator)
  const denominator = BigInt(rate.denominator)
  let remainder = numerator % denominator
  let decimals = ''
  for (let index = 0; remainder !== 0n && index < 18; index++) {
    remainder *= 10n
    decimals += (remainder / denominator).toString()
    remainder %= denominator
  }
  const value =
    remainder === 0n
      ? `${numerator / denominator}${decimals ? `.${decimals}` : ''}`
      : `${numerator}/${denominator}`
  return `${value} USD`
}

const dimensions = [
  ['input', 'Input'],
  ['output', 'Output'],
  ['cache_read', 'Cache Read'],
  ['cache_write', 'Cache write (30 minutes)'],
  ['cache_write_12h', 'Cache write (12 hours)'],
] as const

export function CacheWritePrices({ model }: { model: PricingModel }) {
  const { t } = useTranslation()
  const groups = Object.entries(model.group_cache_write_12h_tiers ?? {}).filter(
    ([group]) => model.enable_groups.includes(group)
  )
  if (groups.length === 0) return null
  return (
    <section className='space-y-3' aria-label={t('Cache lifetime prices')}>
      <h3 className='text-sm font-semibold'>{t('Cache lifetime prices')}</h3>
      <p className='text-muted-foreground text-xs leading-5'>
        {t(
          'Rates are per million tokens and include the selected group and any verified account discount. The full input length selects the tier.'
        )}
      </p>
      {groups.map(([group, tiers]) => (
        <div key={group} className='space-y-3'>
          <GroupBadge group={group} size='sm' />
          {tiers.map((tier, index) => (
            <div
              key={tier.up_to_input_tokens ?? 'unlimited'}
              className='bg-muted/20 rounded-lg border p-3'
            >
              <p className='mb-2 text-xs font-medium'>
                {tier.up_to_input_tokens === null
                  ? index === 0
                    ? t('All input lengths')
                    : t('Input above {{lower}} tokens', {
                        lower: tiers[index - 1]?.up_to_input_tokens,
                      })
                  : index === 0
                    ? t('Input up to {{upper}} tokens', {
                        upper: tier.up_to_input_tokens,
                      })
                    : t('Input above {{lower}} and up to {{upper}} tokens', {
                        lower: tiers[index - 1]?.up_to_input_tokens,
                        upper: tier.up_to_input_tokens,
                      })}
              </p>
              <dl className='grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-x-4 gap-y-2 text-xs'>
                {dimensions.map(([dimension, label]) => (
                  <div key={dimension} className='contents'>
                    <dt className='text-muted-foreground'>{t(label)}</dt>
                    <dd className='text-right font-mono break-all tabular-nums'>
                      {formatCacheWriteRate(tier[dimension])}
                    </dd>
                  </div>
                ))}
              </dl>
            </div>
          ))}
        </div>
      ))}
    </section>
  )
}
