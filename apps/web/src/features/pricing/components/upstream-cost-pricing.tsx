/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import {
  formatUpstreamMultiplier,
  getUpstreamCostPrice,
} from '../lib/upstream-cost'
import type { PricingModel } from '../types'
import { PricingContextLabel } from './pricing-context-label'

export function UpstreamCostPricing(props: {
  model: PricingModel
  selectedGroup?: string
  detailed?: boolean
}) {
  const { t } = useTranslation()
  const rule = getUpstreamCostPrice(props.model, props.selectedGroup)
  if (!rule) return null
  return (
    <div className='min-w-0 space-y-1 text-xs leading-5 wrap-anywhere'>
      <div className='font-medium'>{t('Upstream cost')}</div>
      <div className='font-mono tabular-nums'>
        {t('Credits = ceil(USD cost × 500,000 × {{multiplier}})', {
          multiplier: formatUpstreamMultiplier(rule.price),
        })}
      </div>
      {rule.reason && (
        <p className='forge-price-warning-text'>
          {rule.reason === 'upstream_budget_contract_required'
            ? t('Text calls blocked: supplier budget guarantee required.')
            : t('Text route unavailable.')}
        </p>
      )}
      {props.detailed && (
        <>
          <PricingContextLabel model={props.model} showDiscountNote />
          <p className='text-muted-foreground'>
            {t(
              'Final usage.cost includes the router charges. Token and tool fees are not added again. Missing cost stays pending; explicit zero releases the hold.'
            )}
          </p>
        </>
      )}
    </div>
  )
}
