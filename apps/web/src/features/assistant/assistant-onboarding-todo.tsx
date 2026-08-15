/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import {
  ArrowRight01Icon,
  CheckmarkCircle02Icon,
  CircleIcon,
  ReloadIcon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'

import { getL1OnboardingTodo, type L1OnboardingTodo } from './api'
import { getAssistantOnboardingTodoSteps } from './assistant-onboarding-todo-state'

export function AssistantOnboardingTodo(props: {
  userId: number
  enabled: boolean
  presentation?: 'default' | 'compact'
  onOpenKey: () => void
  onOpenSetup: () => void
}) {
  const { t } = useTranslation()
  const todoQuery = useQuery({
    queryKey: ['assistant-onboarding-todo', props.userId],
    queryFn: getL1OnboardingTodo,
    enabled: props.enabled,
    staleTime: 15_000,
    refetchOnWindowFocus: true,
    retry: false,
  })

  if (!props.enabled || todoQuery.isLoading) return null
  if (todoQuery.isError || !todoQuery.data?.eligibility.eligible) return null

  const steps = getAssistantOnboardingTodoSteps(todoQuery.data)
  const completedCount = steps.filter((step) => step.complete).length
  const isComplete = todoQuery.data.status === 'completed'
  const compact = props.presentation === 'compact'
  const labels: Record<L1OnboardingTodo['steps'][number]['id'], string> = {
    create_api_key: t('Create API key'),
    install_client: t('Install a client'),
    configure_client: t('Configure the client'),
    first_successful_response: t('Receive a successful response'),
  }

  return (
    <Card
      size='sm'
      className={
        compact
          ? 'mx-auto mt-2 w-full max-w-3xl shrink-0 rounded-none border-x-0 border-t-0 bg-transparent shadow-none sm:mt-3'
          : 'mx-3 mt-3 shrink-0 sm:mx-4'
      }
      data-testid='assistant-onboarding-todo'
      data-presentation={compact ? 'compact' : 'default'}
    >
      <CardHeader className='gap-1.5 pb-3'>
        <div className='flex items-center justify-between gap-3'>
          <CardTitle className='text-sm'>{t('First-use checklist')}</CardTitle>
          <div className='flex items-center gap-1.5'>
            <Badge variant={isComplete ? 'secondary' : 'outline'}>
              {completedCount}/{steps.length}
            </Badge>
            <Button
              type='button'
              variant='ghost'
              size='icon-xs'
              aria-label={t('Refresh')}
              title={t('Refresh')}
              onClick={() => void todoQuery.refetch()}
            >
              <HugeiconsIcon
                icon={ReloadIcon}
                strokeWidth={2}
                aria-hidden='true'
              />
            </Button>
          </div>
        </div>
        <p className='text-muted-foreground text-xs leading-5'>
          {t('Only verified account activity can complete these steps.')}
        </p>
      </CardHeader>
      <CardContent
        className={
          compact ? 'grid gap-2 pt-0 sm:grid-cols-2' : 'grid gap-2 pt-0'
        }
      >
        {steps.map((step) => {
          let action: (() => void) | undefined
          if (step.id === 'create_api_key') action = props.onOpenKey
          if (step.id === 'install_client' || step.id === 'configure_client') {
            action = props.onOpenSetup
          }
          const actionLabel =
            step.id === 'create_api_key'
              ? t('Create API key')
              : t('Open setup guide')

          return (
            <div
              className='flex items-start gap-2.5 text-sm'
              data-testid={`assistant-onboarding-step-${step.id}`}
              key={step.id}
            >
              <HugeiconsIcon
                icon={step.complete ? CheckmarkCircle02Icon : CircleIcon}
                className={
                  step.complete
                    ? 'text-success mt-0.5 size-4 shrink-0'
                    : 'text-muted-foreground mt-0.5 size-4 shrink-0'
                }
                strokeWidth={2}
                aria-hidden='true'
              />
              <div className='min-w-0 flex-1'>
                <p className={step.complete ? 'text-muted-foreground' : ''}>
                  {labels[step.id]}
                </p>
                {!step.complete && step.available && action ? (
                  <Button
                    type='button'
                    variant='ghost'
                    size='sm'
                    className='mt-1 h-7 px-2 text-xs'
                    onClick={action}
                  >
                    {actionLabel}
                    <HugeiconsIcon
                      icon={ArrowRight01Icon}
                      strokeWidth={2}
                      data-icon='inline-end'
                      aria-hidden='true'
                    />
                  </Button>
                ) : null}
                {!step.complete && !step.available ? (
                  <p className='text-muted-foreground mt-0.5 text-xs'>
                    {t('Complete the previous step first.')}
                  </p>
                ) : null}
              </div>
            </div>
          )
        })}
        {isComplete ? (
          <p className='text-muted-foreground pt-1 text-xs'>
            {t('Setup complete')}
          </p>
        ) : null}
      </CardContent>
    </Card>
  )
}
