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
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Check,
  CheckCircle2,
  CircleAlert,
  MessageSquareText,
  RefreshCw,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Empty,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Textarea } from '@/components/ui/textarea'
import {
  getAssistantIntentSummary,
  getAssistantFundingSummary,
  getAssistantProfileSummary,
  listAssistantHandoffs,
  resolveAssistantHandoff,
  type AssistantHandoff,
  type AssistantIntentSummary,
  type AssistantProfileSummary,
} from '@/features/assistant/api'
import { toIntlLocale } from '@/i18n/languages'

const PENDING_HANDOFFS_QUERY_KEY = [
  'assistant-admin-handoffs',
  'pending',
] as const
const RESOLVED_HANDOFFS_QUERY_KEY = [
  'assistant-admin-handoffs',
  'resolved',
] as const
const INTENT_SUMMARY_QUERY_KEY = ['assistant-admin-intents', 30] as const
const PROFILE_SUMMARY_QUERY_KEY = ['assistant-admin-profiles', 30] as const
const FUNDING_SUMMARY_QUERY_KEY = ['assistant-admin-funding', 30] as const
const EMPTY_INTENTS: AssistantIntentSummary[] = []
const EMPTY_PROFILES: AssistantProfileSummary[] = []

const INTENT_LABELS: Record<string, string> = {
  onboarding: 'Onboarding and L1',
  plan_purchase: 'Plans and purchase',
  api_key: 'API keys',
  client_setup: 'Client setup',
  cost: 'Cost calculation',
  bounty: 'Open-source bounties',
  human_support: 'Human support',
  other: 'Other questions',
}

const PROFILE_LABELS: Record<string, string> = {
  technical_cost_sensitive: 'Technical cost-sensitive',
  guided_buyer: 'Guided buyer',
  promotion_seeker: 'Promotion seeker',
  security_risk: 'Security-sensitive',
  production_operator: 'Production operator',
  privacy_conscious: 'Privacy-conscious',
  mobile_accessibility: 'Mobile accessibility',
  normal_user: 'Normal user',
  unknown: 'Unknown profile',
}

function isNotFound(error: unknown): boolean {
  return (
    (error as { response?: { status?: number } } | null)?.response?.status ===
    404
  )
}

function HandoffsSkeleton() {
  return (
    <div className='grid gap-3' aria-hidden='true'>
      {[1, 2].map((key) => (
        <div key={key} className='space-y-3 rounded-lg border p-4'>
          <div className='flex justify-between gap-3'>
            <div className='space-y-2'>
              <Skeleton className='h-4 w-28' />
              <Skeleton className='h-3 w-52' />
            </div>
            <Skeleton className='h-5 w-16' />
          </div>
          <Skeleton className='h-12 w-full' />
        </div>
      ))}
    </div>
  )
}

export function AssistantLeadsPanel() {
  const { t, i18n } = useTranslation()
  const intlLocale = toIntlLocale(i18n.language)
  const queryClient = useQueryClient()
  const [notes, setNotes] = useState<Record<number, string>>({})

  const pendingQuery = useQuery({
    queryKey: PENDING_HANDOFFS_QUERY_KEY,
    queryFn: () => listAssistantHandoffs('pending'),
    staleTime: 30_000,
    retry: false,
  })
  const resolvedQuery = useQuery({
    queryKey: RESOLVED_HANDOFFS_QUERY_KEY,
    queryFn: () => listAssistantHandoffs('resolved'),
    staleTime: 30_000,
    retry: false,
  })
  const intentsQuery = useQuery({
    queryKey: INTENT_SUMMARY_QUERY_KEY,
    queryFn: () => getAssistantIntentSummary(30),
    staleTime: 30_000,
    retry: false,
  })
  const profilesQuery = useQuery({
    queryKey: PROFILE_SUMMARY_QUERY_KEY,
    queryFn: () => getAssistantProfileSummary(30),
    staleTime: 30_000,
    retry: false,
  })
  const fundingQuery = useQuery({
    queryKey: FUNDING_SUMMARY_QUERY_KEY,
    queryFn: () => getAssistantFundingSummary(30),
    staleTime: 30_000,
    retry: false,
  })

  const pending = pendingQuery.data ?? []
  const resolved = resolvedQuery.data ?? []
  const intents = intentsQuery.data ?? EMPTY_INTENTS
  const profiles = profilesQuery.data ?? EMPTY_PROFILES
  const funding = fundingQuery.data
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(intlLocale),
    [intlLocale]
  )
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(intlLocale, {
        style: 'currency',
        currency: 'USD',
        maximumFractionDigits: 4,
      }),
    [intlLocale]
  )
  const totalIntents = useMemo(
    () => intents.reduce((total, item) => total + item.count, 0),
    [intents]
  )
  const totalProfiles = useMemo(
    () => profiles.reduce((total, item) => total + item.count, 0),
    [profiles]
  )
  const dateTimeFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(intlLocale, {
        dateStyle: 'medium',
        timeStyle: 'short',
      }),
    [intlLocale]
  )

  const resolveMutation = useMutation({
    mutationFn: ({
      handoff,
      note,
    }: {
      handoff: AssistantHandoff
      note: string
    }) => resolveAssistantHandoff(handoff.id, note),
    onSuccess: (updated, { handoff }) => {
      queryClient.setQueryData<AssistantHandoff[]>(
        PENDING_HANDOFFS_QUERY_KEY,
        (current = []) => current.filter((item) => item.id !== handoff.id)
      )
      queryClient.setQueryData<AssistantHandoff[]>(
        RESOLVED_HANDOFFS_QUERY_KEY,
        (current = []) => [
          {
            ...handoff,
            ...updated,
            username: updated.username ?? handoff.username,
            email: updated.email ?? handoff.email,
          },
          ...current.filter((item) => item.id !== handoff.id),
        ]
      )
      setNotes((current) => {
        const next = { ...current }
        delete next[handoff.id]
        return next
      })
      toast.success(t('Support request resolved'))
      void Promise.all([
        queryClient.invalidateQueries({ queryKey: PENDING_HANDOFFS_QUERY_KEY }),
        queryClient.invalidateQueries({
          queryKey: RESOLVED_HANDOFFS_QUERY_KEY,
        }),
      ])
    },
    onError: (error) => {
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to resolve support request')
      )
    },
  })

  const requiredQueries = [pendingQuery, resolvedQuery, intentsQuery]
  if (requiredQueries.some((query) => isNotFound(query.error))) return null
  const queries = [...requiredQueries, profilesQuery, fundingQuery]

  const firstError = queries.find(
    (query) => query.isError && !isNotFound(query.error)
  )?.error
  const isRefreshing = queries.some((query) => query.isFetching)
  const refresh = () =>
    Promise.all(queries.map((query) => query.refetch())).then(() => undefined)

  const renderPending = () => {
    if (pendingQuery.isLoading) return <HandoffsSkeleton />
    if (pending.length === 0) {
      return (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant='icon'>
              <MessageSquareText aria-hidden='true' />
            </EmptyMedia>
            <EmptyTitle>{t('No pending human-support requests.')}</EmptyTitle>
          </EmptyHeader>
        </Empty>
      )
    }
    return (
      <div className='grid gap-3'>
        {pending.map((handoff) => {
          const isResolving =
            resolveMutation.isPending &&
            resolveMutation.variables?.handoff.id === handoff.id
          return (
            <article
              key={handoff.id}
              className='bg-background rounded-lg border p-4'
            >
              <div className='flex flex-wrap items-start justify-between gap-3'>
                <div className='min-w-0'>
                  <p className='font-medium'>{handoff.username}</p>
                  <p className='text-muted-foreground text-xs'>
                    {handoff.email || t('No email provided')} ·{' '}
                    {dateTimeFormatter.format(
                      new Date(handoff.created_at * 1000)
                    )}
                  </p>
                </div>
                <Badge variant='outline'>{t('Pending')}</Badge>
              </div>
              <p className='mt-3 text-sm whitespace-pre-wrap'>
                {handoff.message}
              </p>
              <Textarea
                className='mt-3'
                rows={2}
                maxLength={2000}
                placeholder={t('Optional resolution note for the user')}
                value={notes[handoff.id] ?? ''}
                onChange={(event) =>
                  setNotes((current) => ({
                    ...current,
                    [handoff.id]: event.target.value,
                  }))
                }
              />
              <div className='mt-3 flex justify-end'>
                <Button
                  size='sm'
                  onClick={() =>
                    resolveMutation.mutate({
                      handoff,
                      note: notes[handoff.id] ?? '',
                    })
                  }
                  disabled={resolveMutation.isPending}
                >
                  {isResolving ? (
                    <RefreshCw
                      data-icon='inline-start'
                      className='animate-spin'
                      aria-hidden='true'
                    />
                  ) : (
                    <Check data-icon='inline-start' aria-hidden='true' />
                  )}
                  {t('Mark resolved')}
                </Button>
              </div>
            </article>
          )
        })}
      </div>
    )
  }

  const renderResolved = () => {
    if (resolvedQuery.isLoading) return <HandoffsSkeleton />
    if (resolved.length === 0) {
      return (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant='icon'>
              <CheckCircle2 aria-hidden='true' />
            </EmptyMedia>
            <EmptyTitle>{t('No resolved human-support requests.')}</EmptyTitle>
          </EmptyHeader>
        </Empty>
      )
    }
    return (
      <div className='grid gap-3'>
        {resolved.map((handoff) => (
          <article
            key={handoff.id}
            className='bg-background rounded-lg border p-4'
          >
            <div className='flex flex-wrap items-start justify-between gap-3'>
              <div className='min-w-0'>
                <p className='font-medium'>{handoff.username}</p>
                <p className='text-muted-foreground text-xs'>
                  {handoff.email || t('No email provided')} ·{' '}
                  {dateTimeFormatter.format(
                    new Date(handoff.created_at * 1000)
                  )}
                </p>
              </div>
              <Badge variant='secondary'>{t('Resolved')}</Badge>
            </div>
            <p className='mt-3 text-sm whitespace-pre-wrap'>
              {handoff.message}
            </p>
            <Separator className='my-3' />
            <p className='text-xs font-medium'>
              {t('Administrator resolution')}
            </p>
            <p className='text-muted-foreground mt-1 text-sm whitespace-pre-wrap'>
              {handoff.admin_note || t('No note provided.')}
            </p>
            {handoff.resolved_at > 0 ? (
              <p className='text-muted-foreground mt-2 text-xs'>
                {t('Resolved at')}:{' '}
                {dateTimeFormatter.format(new Date(handoff.resolved_at * 1000))}
              </p>
            ) : null}
          </article>
        ))}
      </div>
    )
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className='flex items-center gap-2'>
          <MessageSquareText className='size-4' aria-hidden='true' />
          {t('AI assistant leads')}
          <Badge variant='secondary'>{pending.length}</Badge>
        </CardTitle>
        <CardDescription>
          {t(
            'Review explicit human-support requests and recent privacy-minimized intent counts.'
          )}
        </CardDescription>
        <CardAction>
          <Button
            variant='outline'
            size='sm'
            onClick={() => void refresh()}
            disabled={isRefreshing}
          >
            <RefreshCw
              data-icon='inline-start'
              className={isRefreshing ? 'animate-spin' : undefined}
              aria-hidden='true'
            />
            {t('Refresh')}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className='grid gap-4'>
        {firstError ? (
          <Alert variant='destructive'>
            <CircleAlert aria-hidden='true' />
            <AlertTitle>{t('Unable to load assistant leads')}</AlertTitle>
            <AlertDescription>
              {firstError instanceof Error
                ? firstError.message
                : t('Unable to load assistant leads')}
            </AlertDescription>
            <AlertAction>
              <Button
                variant='outline'
                size='sm'
                onClick={() => void refresh()}
                disabled={isRefreshing}
              >
                {t('Retry')}
              </Button>
            </AlertAction>
          </Alert>
        ) : null}

        <div className='flex flex-wrap gap-2' aria-label={t('Intent summary')}>
          {intentsQuery.isLoading ? (
            <Skeleton className='h-5 w-36' />
          ) : (
            <>
              <Badge variant='outline'>
                {t('{{count}} questions in 30 days', { count: totalIntents })}
              </Badge>
              {intents.map((item) => (
                <Badge key={item.intent} variant='secondary'>
                  {t(INTENT_LABELS[item.intent] ?? 'Other questions')}:{' '}
                  {item.count}
                </Badge>
              ))}
            </>
          )}
        </div>

        <div
          className='flex flex-wrap gap-2'
          aria-label={t('Customer profiles')}
        >
          {profilesQuery.isLoading ? (
            <Skeleton className='h-5 w-40' />
          ) : profilesQuery.isError &&
            !isNotFound(profilesQuery.error) ? null : (
            <>
              <Badge variant='outline'>
                {t('{{count}} profile signals in 30 days', {
                  count: totalProfiles,
                })}
              </Badge>
              {profiles.map((item) => (
                <Badge key={item.profile} variant='secondary'>
                  {t(PROFILE_LABELS[item.profile] ?? 'Unknown profile')}:{' '}
                  {item.count}
                </Badge>
              ))}
            </>
          )}
        </div>

        <Card className='bg-muted/20'>
          <CardHeader className='pb-3'>
            <CardTitle className='text-base'>
              {t('AI assistant')} · {t('Cost')}
            </CardTitle>
            <CardDescription>
              {t(
                'AI customer-service token usage is charged to the super administrator account, not your wallet.'
              )}
            </CardDescription>
          </CardHeader>
          <CardContent>
            {fundingQuery.isLoading ? (
              <div className='grid gap-3 sm:grid-cols-2 lg:grid-cols-4'>
                {[1, 2, 3, 4].map((key) => (
                  <Skeleton key={key} className='h-16 w-full' />
                ))}
              </div>
            ) : fundingQuery.isError && !isNotFound(fundingQuery.error) ? (
              <p className='text-muted-foreground text-sm'>
                {fundingQuery.error instanceof Error
                  ? fundingQuery.error.message
                  : t('Unable to load assistant leads')}
              </p>
            ) : funding ? (
              <div className='grid gap-3 sm:grid-cols-2 lg:grid-cols-4'>
                <div className='rounded-md border p-3'>
                  <p className='text-muted-foreground text-xs'>{t('Cost')}</p>
                  <p className='mt-1 text-lg font-semibold'>
                    {currencyFormatter.format(funding.cost_usd)}
                  </p>
                </div>
                <div className='rounded-md border p-3'>
                  <p className='text-muted-foreground text-xs'>
                    {t('Requests')}
                  </p>
                  <p className='mt-1 text-lg font-semibold'>
                    {numberFormatter.format(funding.requests)}
                  </p>
                </div>
                <div className='rounded-md border p-3'>
                  <p className='text-muted-foreground text-xs'>
                    {t('Total tokens')}
                  </p>
                  <p className='mt-1 text-lg font-semibold'>
                    {numberFormatter.format(funding.total_tokens)}
                  </p>
                </div>
                <div className='rounded-md border p-3'>
                  <p className='text-muted-foreground text-xs'>
                    {t('Remaining quota')}
                  </p>
                  <p className='mt-1 text-lg font-semibold'>
                    {currencyFormatter.format(funding.remaining_usd)}
                  </p>
                  <p className='text-muted-foreground text-xs'>
                    {numberFormatter.format(funding.remaining_quota)}{' '}
                    {t('Remaining quota units')}
                  </p>
                </div>
              </div>
            ) : (
              <p className='text-muted-foreground text-sm'>
                {t('No recent usage')}
              </p>
            )}
          </CardContent>
        </Card>

        <Tabs defaultValue='pending'>
          <TabsList>
            <TabsTrigger value='pending'>
              {t('Pending')}
              <Badge variant='secondary'>{pending.length}</Badge>
            </TabsTrigger>
            <TabsTrigger value='resolved'>
              {t('Resolved')}
              <Badge variant='secondary'>{resolved.length}</Badge>
            </TabsTrigger>
          </TabsList>
          <TabsContent value='pending'>{renderPending()}</TabsContent>
          <TabsContent value='resolved'>{renderResolved()}</TabsContent>
        </Tabs>
      </CardContent>
    </Card>
  )
}
