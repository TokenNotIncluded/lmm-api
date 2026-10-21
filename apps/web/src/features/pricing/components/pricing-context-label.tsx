/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

import {
  getAccountPriceMultiplier,
  getPricingBasisLabel,
} from '../lib/pricing-context'
import type { PricingModel } from '../types'

export function PricingContextLabel(props: {
  model: PricingModel
  legacyLabel?: string
  className?: string
  showDiscountNote?: boolean
}) {
  const { t } = useTranslation()
  const multiplier = getAccountPriceMultiplier(props.model)
  return (
    <span className={cn('block', props.className)}>
      <span>{t(getPricingBasisLabel(props.model, props.legacyLabel))}</span>
      {multiplier !== null && (
        <span className='mt-1 block text-xs font-normal'>
          {t('Account discount')}:{' '}
          {t('{{multiplier}} of group base price', { multiplier })}
          {props.showDiscountNote && (
            <span className='mt-1 block'>
              {t('Prices include your account discount.')}
            </span>
          )}
        </span>
      )}
    </span>
  )
}
