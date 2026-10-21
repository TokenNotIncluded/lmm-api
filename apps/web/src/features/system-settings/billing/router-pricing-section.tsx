/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { useCopyToClipboard } from '@/hooks/use-copy-to-clipboard'
import { useAuthStore } from '@/stores/auth-store'

import { getPricing, pricingQueryKey } from '../../pricing/api'
import { UpstreamCostPricing } from '../../pricing/components/upstream-cost-pricing'
import { ROUTER_MODELS, routerPriceDraft } from './router-price-draft'

export function RouterPricingSection() {
  const { t } = useTranslation()
  const { user, session } = useAuthStore((state) => state.auth)
  const { copyToClipboard } = useCopyToClipboard()
  const query = useQuery({
    queryKey: pricingQueryKey(user?.id, session?.sid),
    queryFn: ({ signal }) =>
      getPricing(signal, { userId: user?.id, sessionId: session?.sid }),
  })
  const [id, setId] = useState('1')
  const [model, setModel] = useState<string>(ROUTER_MODELS[0])
  const [group, setGroup] = useState('')
  const [multiplier, setMultiplier] = useState('1')
  if (query.isPending) {
    return <LoadingState message={t('Loading settings...')} />
  }
  if (query.isError) return <ErrorState onRetry={() => void query.refetch()} />
  const groups = Object.keys(query.data.usable_group)
  const selectedGroup = groups.includes(group) ? group : groups[0] || ''
  const draft = routerPriceDraft(id, model, selectedGroup, multiplier, groups)
  const configured = query.data.data.filter(
    (row) => Object.keys(row.group_upstream_cost_prices ?? {}).length
  )
  return (
    <section className='space-y-6 p-4 sm:p-6'>
      <p className='text-muted-foreground text-sm leading-6'>
        {t(
          'Router rules use actual upstream USD cost. External text calls remain blocked until the supplier guarantees the request budget.'
        )}
      </p>
      <div className='space-y-4'>
        <h3 className='font-medium'>{t('Active router rules')}</h3>
        {configured.length === 0 && (
          <p className='text-muted-foreground text-sm'>
            {t('No upstream-cost rules in the current catalog.')}
          </p>
        )}
        {configured.flatMap((row) =>
          Object.keys(row.group_upstream_cost_prices ?? {}).map((name) => (
            <div
              key={`${row.model_name}:${name}`}
              className='space-y-2 rounded-lg border p-4'
            >
              <div className='text-sm font-medium wrap-anywhere'>
                {row.model_name} · {name}
              </div>
              <UpstreamCostPricing model={row} selectedGroup={name} detailed />
            </div>
          ))
        )}
      </div>
      <div className='space-y-4 border-t pt-6'>
        <h3 className='font-medium'>{t('Configuration preview')}</h3>
        <p className='text-muted-foreground text-sm leading-6'>
          {t(
            'Preview only. Merge this rule into the Core owner configuration, preserve existing rule IDs, advance the price version, and validate before restarting. This page does not save or activate rules.'
          )}
        </p>
        <div className='grid gap-4 sm:grid-cols-2'>
          <div className='space-y-2'>
            <Label htmlFor='router-rule-id'>{t('Rule ID')}</Label>
            <Input
              id='router-rule-id'
              inputMode='numeric'
              value={id}
              onChange={(event) => setId(event.target.value)}
            />
          </div>
          <div className='space-y-2'>
            <Label htmlFor='router-model'>{t('Model')}</Label>
            <NativeSelect
              id='router-model'
              className='w-full'
              value={model}
              onChange={(event) => setModel(event.target.value)}
            >
              {ROUTER_MODELS.map((name) => (
                <NativeSelectOption key={name}>{name}</NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          <div className='space-y-2'>
            <Label htmlFor='router-group'>{t('Group')}</Label>
            <NativeSelect
              id='router-group'
              className='w-full'
              value={selectedGroup}
              onChange={(event) => setGroup(event.target.value)}
            >
              {groups.map((name) => (
                <NativeSelectOption key={name}>{name}</NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          <div className='space-y-2'>
            <Label htmlFor='router-multiplier'>
              {t('Group cost multiplier')}
            </Label>
            <Input
              id='router-multiplier'
              inputMode='decimal'
              value={multiplier}
              onChange={(event) => setMultiplier(event.target.value)}
            />
          </div>
        </div>
        {draft ? (
          <pre className='bg-muted overflow-auto rounded-lg p-4 text-xs'>
            {draft}
          </pre>
        ) : (
          <p className='text-destructive text-sm'>
            {t(
              'Enter a valid rule ID, catalog group and multiplier above 0 and at most 1000 (up to 6 decimal places).'
            )}
          </p>
        )}
        <Button
          variant='outline'
          disabled={!draft}
          onClick={() => draft && copyToClipboard(draft)}
        >
          {t('Copy configuration')}
        </Button>
      </div>
    </section>
  )
}
