/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import { CheckmarkCircle02Icon, GiftIcon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'

import { claimAssistantWeeklyDiscount, getAssistantWeeklyDiscount } from './api'
import { copyAssistantText } from './assistant-clipboard'

export function AssistantWeeklyDiscount(props: { enabled: boolean }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [claiming, setClaiming] = useState(false)
  const discountQuery = useQuery({
    queryKey: ['assistant-weekly-discount'],
    queryFn: getAssistantWeeklyDiscount,
    enabled: props.enabled,
    staleTime: 15_000,
    retry: false,
  })
  const discount = discountQuery.data
  if (!props.enabled || !discount) return null

  const claim = async () => {
    if (claiming || discount.status !== 'offered') return
    setClaiming(true)
    try {
      const result = await claimAssistantWeeklyDiscount()
      queryClient.setQueryData(['assistant-weekly-discount'], result.discount)
      toast.success(t('Weekly discount claimed'))
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to claim weekly discount')
      )
    } finally {
      setClaiming(false)
    }
  }

  let action: ReactNode = (
    <span className='text-muted-foreground text-xs'>
      {t("This week's decision is used")}
    </span>
  )
  if (discount.status === 'offered') {
    action = (
      <Button
        type='button'
        size='sm'
        onClick={() => void claim()}
        disabled={claiming}
      >
        {claiming ? t('Claiming...') : t('Claim discount code')}
      </Button>
    )
  } else if (discount.status === 'claimed') {
    action = (
      <div className='flex min-w-0 items-center gap-2'>
        <code className='bg-muted max-w-44 truncate rounded px-2 py-1 text-xs'>
          {discount.code ?? t('Code hidden')}
        </code>
        {discount.code ? (
          <Button
            type='button'
            size='sm'
            variant='outline'
            onClick={async () => {
              const copied = await copyAssistantText(
                discount.code ?? '',
                navigator.clipboard
              )
              if (copied) {
                toast.success(t('Discount code copied'))
              } else {
                toast.error(t('Copy failed'))
              }
            }}
          >
            {t('Copy')}
          </Button>
        ) : null}
      </div>
    )
  }

  return (
    <section
      className='border-border/60 bg-accent/20 my-2 grid gap-3 border-y px-1 py-5 sm:grid-cols-[1fr_auto] sm:items-center sm:px-4'
      data-testid='assistant-weekly-discount'
    >
      <div className='flex min-w-0 gap-3'>
        <HugeiconsIcon
          icon={
            discount.status === 'claimed' ? CheckmarkCircle02Icon : GiftIcon
          }
          className={
            discount.status === 'claimed'
              ? 'text-success mt-0.5 size-5'
              : 'mt-0.5 size-5'
          }
          strokeWidth={2}
          aria-hidden='true'
        />
        <div className='min-w-0'>
          <p className='text-sm font-medium'>
            {t('Weekly recharge discount')} · {discount.discount_percent}%
          </p>
          <p className='text-muted-foreground mt-1 text-xs leading-5'>
            {discount.reason}
          </p>
          <p className='text-muted-foreground mt-1 text-xs'>
            {t('One claim per UTC week')}
          </p>
        </div>
      </div>
      {action}
    </section>
  )
}
