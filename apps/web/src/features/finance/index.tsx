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
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { getRouteApi, Link, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, ReceiptText, RefreshCw, WalletCards } from 'lucide-react'
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { toast } from 'sonner'

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'

import {
  createFinanceExpense,
  getFinanceEntries,
  getFinanceOverview,
  getFinanceUser,
  type FinanceLedgerEntry,
  updateFinancePaymentMethod,
  type FinancePaymentMethod,
  type FinanceUserMetric,
} from './api'
import {
  financeLedgerUserFilter,
  financeLedgerUserSearch,
} from './ledger-user-filter'
import {
  paymentMethodSummary,
  type PaymentMethodSummary,
} from './payment-method-metrics'
import { userNetRevenueMicros } from './user-finance-metrics'

const route = getRouteApi('/_authenticated/finance/')
const WINDOWS = [7, 30, 90] as const

function money(micros: number, currency = 'USD') {
  return new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency,
    maximumFractionDigits: 2,
  }).format(micros / 1_000_000)
}

function compact(value: number) {
  return new Intl.NumberFormat(undefined, { notation: 'compact' }).format(value)
}

function Metric({
  label,
  value,
  tone,
  detail,
}: {
  label: string
  value: string
  tone?: 'positive' | 'negative'
  detail?: string
}) {
  return (
    <div className='min-w-0 px-4 py-4 sm:px-5 sm:py-5'>
      <p className='text-muted-foreground text-xs'>{label}</p>
      <p
        className={cn(
          'mt-2 truncate text-2xl font-semibold tracking-tight',
          tone === 'positive' && 'text-emerald-600 dark:text-emerald-400',
          tone === 'negative' && 'text-rose-600 dark:text-rose-400'
        )}
      >
        {value}
      </p>
      {detail ? (
        <p className='text-muted-foreground mt-1 text-xs'>{detail}</p>
      ) : null}
    </div>
  )
}

function PaymentMethodRow({
  method,
  summary,
  currency,
  onChange,
  onView,
  viewing,
}: {
  method: FinancePaymentMethod
  summary: PaymentMethodSummary
  currency: string
  onChange: (value: Partial<FinancePaymentMethod>) => void
  onView: () => void
  viewing: boolean
}) {
  const { t } = useTranslation()
  return (
    <div className='flex items-center gap-3 py-3'>
      <WalletCards
        className='text-muted-foreground size-4 shrink-0'
        aria-hidden='true'
      />
      <div className='min-w-0 flex-1'>
        <Button
          type='button'
          variant='link'
          size='sm'
          className='h-auto max-w-full justify-start px-0 py-0 text-left text-sm font-medium'
          aria-pressed={viewing}
          onClick={onView}
        >
          {method.label || method.method}
        </Button>
        <p className='text-muted-foreground truncate text-xs'>
          {method.method}
        </p>
        {summary.revenueMicros !== 0 || summary.refundMicros !== 0 ? (
          <p className='text-muted-foreground mt-1 text-xs tabular-nums'>
            {t('Revenue')}: {money(summary.revenueMicros, currency)}
            {summary.refundMicros > 0
              ? ` · ${t('Refund')}: ${money(summary.refundMicros, currency)}`
              : ''}
            {summary.refundMicros > 0
              ? ` = ${money(summary.netRevenueMicros, currency)}`
              : ''}
          </p>
        ) : null}
      </div>
      <div className='text-muted-foreground flex flex-wrap items-center justify-end gap-x-3 gap-y-1 text-xs'>
        <label className='flex items-center gap-2'>
          <span>{t('Enable')}</span>
          <Switch
            size='sm'
            checked={method.enabled}
            aria-label={`${t('Enable')}: ${method.label || method.method}`}
            onCheckedChange={(checked) => onChange({ enabled: checked })}
          />
        </label>
        <label className='flex items-center gap-2'>
          <span>{t('Include revenue')}</span>
          <Switch
            size='sm'
            checked={method.include_revenue}
            aria-label={`${t('Include revenue')}: ${method.label || method.method}`}
            onCheckedChange={(checked) =>
              onChange({ include_revenue: checked })
            }
          />
        </label>
      </div>
    </div>
  )
}

function UserRow({
  user,
  days,
  paymentMethod,
}: {
  user: FinanceUserMetric
  days: number
  paymentMethod?: string
}) {
  const { t } = useTranslation()
  const title = user.display_name || user.username || `#${user.user_id}`
  const netRevenueMicros = userNetRevenueMicros(
    user.revenue_micros,
    user.refund_micros
  )
  return (
    <Link
      to='/finance'
      search={{ user_id: user.user_id, payment_method: paymentMethod }}
      className='group flex items-center gap-3 py-3'
    >
      <span className='bg-muted text-muted-foreground flex size-8 shrink-0 items-center justify-center rounded-full text-xs'>
        {String(user.user_id).slice(-2)}
      </span>
      <span className='min-w-0 flex-1'>
        <span className='block truncate text-sm font-medium'>{title}</span>
        <span className='text-muted-foreground block text-xs'>
          {user.username && user.display_name ? `${user.username} · ` : ''}#
          {user.user_id} · {compact(user.token_units)} tokens · {user.requests}{' '}
          {t('Requests').toLowerCase()}
        </span>
      </span>
      <span className='text-right'>
        <span className='block text-sm font-medium'>
          {money(netRevenueMicros)}
        </span>
        <span className='text-muted-foreground block text-xs'>
          {user.refund_micros > 0
            ? `${t('Refund')}: ${money(-user.refund_micros)}`
            : `${days}d`}
        </span>
      </span>
    </Link>
  )
}

function UserDetail({
  userId,
  days,
  paymentMethod,
  onViewLedger,
}: {
  userId: number
  days: number
  paymentMethod?: string
  onViewLedger: () => void
}) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const query = useQuery({
    queryKey: ['finance-user', userId, days, paymentMethod],
    queryFn: () => getFinanceUser(userId, days, paymentMethod),
  })
  const overview = query.data?.data
  const user = overview?.users?.[0]
  const netRevenueMicros = userNetRevenueMicros(
    overview?.revenue_micros ?? 0,
    overview?.refund_micros ?? 0
  )
  let detailContent: ReactNode
  if (query.isLoading) {
    detailContent = (
      <p className='text-muted-foreground mt-5 text-sm'>{t('Loading...')}</p>
    )
  } else if (overview) {
    detailContent = (
      <div className='mt-5 grid gap-4 sm:grid-cols-3'>
        <Metric
          label={t('User spending')}
          value={money(netRevenueMicros)}
          detail={
            overview.refund_micros > 0
              ? `${t('Revenue')}: ${money(overview.revenue_micros)} · ${t('Refund')}: ${money(overview.refund_micros)}`
              : undefined
          }
        />
        <Metric
          label={t('Tokens')}
          value={compact(overview.tokens.total_tokens)}
        />
        <Metric
          label={t('Requests')}
          value={compact(overview.tokens.requests)}
        />
      </div>
    )
  } else {
    detailContent = (
      <p className='text-muted-foreground mt-5 text-sm'>{t('No data')}</p>
    )
  }
  return (
    <section className='border-t pt-6' aria-labelledby='finance-user-detail'>
      <div className='flex flex-wrap items-start justify-between gap-3'>
        <div className='min-w-0'>
          <p className='text-muted-foreground text-xs'>{t('User spending')}</p>
          <h3
            id='finance-user-detail'
            className='mt-1 truncate text-lg font-semibold'
          >
            {user?.display_name || user?.username || `#${userId}`}
          </h3>
          <p className='text-muted-foreground mt-1 text-xs'>
            {user?.username || `#${userId}`}
          </p>
        </div>
        <div className='flex shrink-0 items-center gap-1'>
          <Button variant='ghost' size='sm' onClick={onViewLedger}>
            <ReceiptText data-icon='inline-start' aria-hidden='true' />
            {t('Append-only ledger')}
          </Button>
          <Button
            variant='ghost'
            size='sm'
            onClick={() =>
              void navigate({
                to: '/finance',
                search: { payment_method: paymentMethod },
              })
            }
          >
            <ArrowLeft data-icon='inline-start' aria-hidden='true' />
            {t('Back')}
          </Button>
        </div>
      </div>
      {detailContent}
    </section>
  )
}

function LedgerEntriesDialog({
  open,
  onOpenChange,
  days,
  paymentMethod,
  paymentMethodLabel,
  initialUserID,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  days: number
  paymentMethod?: string
  paymentMethodLabel?: string
  initialUserID?: number
}) {
  const { t } = useTranslation()
  const [userIDText, setUserIDText] = useState(() =>
    financeLedgerUserFilter(initialUserID)
  )
  useEffect(() => {
    if (open) setUserIDText(financeLedgerUserFilter(initialUserID))
  }, [initialUserID, open])
  const userID = Number.parseInt(userIDText, 10)
  const query = useInfiniteQuery({
    queryKey: ['finance-ledger-entries', days, paymentMethod, userIDText],
    queryFn: ({ pageParam }) =>
      getFinanceEntries(days, {
        paymentMethod,
        userId: Number.isSafeInteger(userID) && userID > 0 ? userID : undefined,
        beforeOccurredAt: pageParam.occurredAt || undefined,
        beforeId: pageParam.id || undefined,
      }),
    initialPageParam: { occurredAt: 0, id: 0 },
    getNextPageParam: (lastPage) => {
      const occurredAt = lastPage.data?.next_before_occurred_at
      const id = lastPage.data?.next_before_id
      return lastPage.data?.has_more && occurredAt && id
        ? { occurredAt, id }
        : undefined
    },
    enabled: open,
  })
  const entries =
    query.data?.pages.flatMap((page) => page.data?.entries ?? []) ?? []
  const title = paymentMethodLabel || paymentMethod || t('Append-only ledger')

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='flex max-h-[min(44rem,calc(100svh-2rem))] w-[calc(100%-1rem)] max-w-2xl flex-col gap-0 p-0 sm:max-w-2xl'>
        <DialogHeader className='border-b px-5 py-4 pr-12'>
          <DialogTitle className='flex items-center gap-2 text-base'>
            <ReceiptText className='size-4' aria-hidden='true' />
            {title}
          </DialogTitle>
          <DialogDescription className='mt-1 text-xs leading-5'>
            {t(
              'Ledger entries are limited to durable ledger events. Use Financial overview for reconciled revenue.'
            )}
          </DialogDescription>
        </DialogHeader>
        <div className='border-b px-5 py-3'>
          <Input
            value={userIDText}
            inputMode='numeric'
            placeholder={t('User ID')}
            aria-label={t('User ID')}
            onChange={(event) =>
              setUserIDText(event.target.value.replace(/\D/g, ''))
            }
          />
        </div>
        <div className='min-h-0 overflow-y-auto px-5'>
          {query.isLoading ? (
            <p className='text-muted-foreground py-8 text-sm'>
              {t('Loading...')}
            </p>
          ) : null}
          {query.isError ? (
            <div className='flex flex-col items-start gap-3 py-8'>
              <p className='text-destructive text-sm'>
                {t('Unable to load data')}
              </p>
              <Button
                type='button'
                variant='outline'
                size='sm'
                onClick={() => void query.refetch()}
                disabled={query.isFetching}
              >
                {query.isFetching ? t('Loading...') : t('Retry')}
              </Button>
            </div>
          ) : null}
          {!query.isLoading && !query.isError && entries.length === 0 ? (
            <p className='text-muted-foreground py-8 text-sm'>{t('No data')}</p>
          ) : null}
          {entries.map((entry) => (
            <LedgerEntryRow key={entry.id} entry={entry} />
          ))}
          {query.hasNextPage ? (
            <div className='flex justify-center border-t py-4'>
              <Button
                type='button'
                variant='ghost'
                size='sm'
                disabled={query.isFetchingNextPage}
                onClick={() => void query.fetchNextPage()}
              >
                {query.isFetchingNextPage ? t('Loading...') : t('More')}
              </Button>
            </div>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  )
}

function LedgerEntryRow({ entry }: { entry: FinanceLedgerEntry }) {
  const { t } = useTranslation()
  const isCredit = entry.direction > 0
  const userSearch = financeLedgerUserSearch(entry.user_id)
  return (
    <div className='flex items-start justify-between gap-4 border-b py-4 last:border-b-0'>
      <div className='min-w-0'>
        <p className='truncate text-sm font-medium'>
          {entry.category || entry.source_type}
        </p>
        <p className='text-muted-foreground mt-1 truncate text-xs'>
          {entry.payment_method || entry.payment_provider || '—'} ·{' '}
          {userSearch ? (
            <Link
              to='/users'
              search={userSearch}
              className='hover:text-foreground underline-offset-4 hover:underline'
              title={t('View user in user management')}
            >
              {t('User ID')} #{userSearch.filter}
            </Link>
          ) : (
            '—'
          )}
        </p>
        <p className='text-muted-foreground mt-1 text-xs'>
          {new Date(entry.occurred_at * 1000).toLocaleString()}
        </p>
      </div>
      <p
        className={cn(
          'shrink-0 text-sm font-medium tabular-nums',
          isCredit
            ? 'text-emerald-600 dark:text-emerald-400'
            : 'text-rose-600 dark:text-rose-400'
        )}
      >
        {isCredit ? '+' : '-'}
        {money(entry.amount_micros, entry.currency)}
      </p>
    </div>
  )
}

export function Finance() {
  const { t } = useTranslation()
  const search = route.useSearch()
  const queryClient = useQueryClient()
  const navigate = route.useNavigate()
  const [days, setDays] = useState<(typeof WINDOWS)[number]>(30)
  const [expenseOpen, setExpenseOpen] = useState(false)
  const [ledgerOpen, setLedgerOpen] = useState(false)
  const [ledgerPaymentMethod, setLedgerPaymentMethod] = useState<string>()
  const [ledgerPaymentMethodLabel, setLedgerPaymentMethodLabel] =
    useState<string>()
  const [ledgerUserID, setLedgerUserID] = useState<number>()
  const [expenseAmount, setExpenseAmount] = useState('')
  const [expenseCategory, setExpenseCategory] = useState('')
  const [expenseNote, setExpenseNote] = useState('')
  const overviewQuery = useQuery({
    queryKey: ['finance-overview', days, search.payment_method],
    queryFn: () => getFinanceOverview(days, search.payment_method),
  })
  const overview = overviewQuery.data?.data
  const selectedPaymentMethod = search.payment_method
  const selectedPaymentMethodLabel =
    overview?.payment_methods.find(
      (method) => method.method === selectedPaymentMethod
    )?.label || selectedPaymentMethod
  const expenseMutation = useMutation({
    mutationFn: () =>
      createFinanceExpense({
        category: expenseCategory.trim() || 'external',
        amount_micros: Math.round(Number(expenseAmount) * 1_000_000),
        currency: 'USD',
        note: expenseNote.trim() || undefined,
        idempotency_key: `finance-${Date.now()}-${Math.random().toString(36).slice(2)}`,
      }),
    onSuccess: async () => {
      setExpenseAmount('')
      setExpenseCategory('')
      setExpenseNote('')
      setExpenseOpen(false)
      await queryClient.invalidateQueries({ queryKey: ['finance-overview'] })
    },
  })

  const chartData = useMemo(
    () =>
      (overview?.daily ?? []).map((item) => ({
        ...item,
        label: item.date.slice(5),
        revenue: item.revenue_micros / 1_000_000,
        refund: (item.refund_micros ?? 0) / 1_000_000,
        expense: item.expense_micros / 1_000_000,
      })),
    [overview?.daily]
  )

  const updateMethod = async (
    method: FinancePaymentMethod,
    value: Partial<FinancePaymentMethod>
  ) => {
    try {
      const response = await updateFinancePaymentMethod(method.method, value)
      if (!response.success) {
        throw new Error(
          response.message || t('Unable to update payment method')
        )
      }
      await queryClient.invalidateQueries({ queryKey: ['finance-overview'] })
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to update payment method')
      )
    }
  }

  const openLedger = (
    paymentMethod?: string,
    paymentMethodLabel?: string,
    userID?: number
  ) => {
    setLedgerPaymentMethod(paymentMethod)
    setLedgerPaymentMethodLabel(paymentMethodLabel)
    setLedgerUserID(userID)
    setLedgerOpen(true)
  }

  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Cost control')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <Button
          variant='ghost'
          size='sm'
          onClick={() => void overviewQuery.refetch()}
          disabled={overviewQuery.isFetching}
        >
          <RefreshCw
            data-icon='inline-start'
            aria-hidden='true'
            className={cn(overviewQuery.isFetching && 'animate-spin')}
          />
          {t('Refresh')}
        </Button>
      </SectionPageLayout.Actions>
      <SectionPageLayout.Content>
        <div className='mx-auto w-full max-w-7xl space-y-8 pb-8'>
          <div className='flex flex-wrap items-end justify-between gap-4'>
            <div>
              <p className='text-muted-foreground text-sm'>
                {t('Financial overview')}
              </p>
              <p className='text-muted-foreground mt-1 text-xs'>
                {t('Token economy')} · {overview?.currency ?? 'USD'}
                {overview?.sources_bounded ? ` · ${t('Estimated')}` : ''}
              </p>
            </div>
            <div className='flex flex-wrap items-center gap-2'>
              {WINDOWS.map((window) => (
                <Button
                  key={window}
                  variant={days === window ? 'secondary' : 'ghost'}
                  size='sm'
                  onClick={() => setDays(window)}
                >
                  {t(`Past ${window} days`)}
                </Button>
              ))}
              {selectedPaymentMethod ? (
                <div className='text-muted-foreground flex items-center gap-1 text-xs'>
                  <span className='max-w-32 truncate'>
                    {t('Payment method')}: {selectedPaymentMethodLabel}
                  </span>
                  <Button
                    type='button'
                    variant='link'
                    size='sm'
                    className='h-auto px-1 py-0 text-xs'
                    onClick={() =>
                      void navigate({
                        to: '/finance',
                        search: { user_id: undefined },
                      })
                    }
                  >
                    {t('Clear filters')}
                  </Button>
                </div>
              ) : null}
            </div>
          </div>

          {overviewQuery.isError ? (
            <p className='text-destructive text-sm'>
              {t('Unable to load data')}
            </p>
          ) : null}
          <div className='divide-border/70 grid border-y sm:grid-cols-2 sm:divide-x lg:grid-cols-4'>
            <Metric
              label={t('Revenue')}
              value={money(
                overview?.net_revenue_micros ?? overview?.revenue_micros ?? 0
              )}
              detail={
                (overview?.refund_micros ?? 0) > 0
                  ? `${t('Refund')}: ${money(overview?.refund_micros ?? 0)}`
                  : undefined
              }
            />
            <Metric
              label={t('Expenses')}
              value={money(overview?.expense_micros ?? 0)}
            />
            <Metric
              label={t('Profit')}
              value={
                overview?.cost_attribution === 'unavailable_for_payment_method'
                  ? '—'
                  : money(overview?.profit_micros ?? 0)
              }
              tone={
                overview?.cost_attribution === 'unavailable_for_payment_method'
                  ? undefined
                  : (overview?.profit_micros ?? 0) >= 0
                    ? 'positive'
                    : 'negative'
              }
              detail={
                overview?.cost_attribution === 'unavailable_for_payment_method'
                  ? t('Profit is unavailable for a payment-method filter')
                  : `${t('Profit margin')}: ${overview?.revenue_micros ? Math.round((overview.profit_micros / overview.revenue_micros) * 100 * 10) / 10 : 0}%`
              }
            />
            <Metric
              label={t('Token economy')}
              value={
                overview?.cost_attribution === 'unavailable_for_payment_method'
                  ? '—'
                  : compact(overview?.tokens.total_tokens ?? 0)
              }
              detail={
                overview?.cost_attribution === 'unavailable_for_payment_method'
                  ? t('Usage is unavailable for a payment-method filter')
                  : `${compact(overview?.tokens.requests ?? 0)} ${t('Requests').toLowerCase()}`
              }
            />
          </div>

          <div className='grid gap-8 lg:grid-cols-[minmax(0,1fr)_18rem]'>
            <section aria-labelledby='finance-trend-heading'>
              <div className='mb-4 flex items-baseline justify-between gap-3'>
                <h3
                  id='finance-trend-heading'
                  className='text-sm font-semibold'
                >
                  {t('Financial overview')}
                </h3>
                <span className='text-muted-foreground text-xs'>
                  {t('Estimated')}:{' '}
                  {money(overview?.tokens.estimated_cost_micros ?? 0)}
                </span>
              </div>
              <div className='h-64 min-h-0 w-full'>
                <ResponsiveContainer width='100%' height='100%'>
                  <AreaChart
                    data={chartData}
                    margin={{ top: 8, right: 8, left: -18, bottom: 0 }}
                  >
                    <defs>
                      <linearGradient
                        id='finance-revenue'
                        x1='0'
                        y1='0'
                        x2='0'
                        y2='1'
                      >
                        <stop
                          offset='5%'
                          stopColor='var(--chart-1)'
                          stopOpacity={0.24}
                        />
                        <stop
                          offset='95%'
                          stopColor='var(--chart-1)'
                          stopOpacity={0}
                        />
                      </linearGradient>
                      <linearGradient
                        id='finance-expense'
                        x1='0'
                        y1='0'
                        x2='0'
                        y2='1'
                      >
                        <stop
                          offset='5%'
                          stopColor='var(--chart-2)'
                          stopOpacity={0.18}
                        />
                        <stop
                          offset='95%'
                          stopColor='var(--chart-2)'
                          stopOpacity={0}
                        />
                      </linearGradient>
                    </defs>
                    <CartesianGrid vertical={false} stroke='var(--border)' />
                    <XAxis
                      dataKey='label'
                      axisLine={false}
                      tickLine={false}
                      tick={{ fontSize: 11 }}
                    />
                    <YAxis
                      axisLine={false}
                      tickLine={false}
                      tick={{ fontSize: 11 }}
                      tickFormatter={(value) => `$${value}`}
                    />
                    <Tooltip
                      formatter={(value, name) => [
                        `$${Number(value).toFixed(2)}`,
                        name === 'revenue'
                          ? t('Revenue')
                          : name === 'refund'
                            ? t('Refund')
                            : t('Expenses'),
                      ]}
                    />
                    <Area
                      type='monotone'
                      dataKey='revenue'
                      stroke='var(--chart-1)'
                      fill='url(#finance-revenue)'
                      strokeWidth={2}
                    />
                    <Area
                      type='monotone'
                      dataKey='expense'
                      stroke='var(--chart-2)'
                      fill='url(#finance-expense)'
                      strokeWidth={2}
                    />
                    <Area
                      type='monotone'
                      dataKey='refund'
                      stroke='var(--destructive)'
                      fill='none'
                      strokeWidth={2}
                      strokeDasharray='4 4'
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
            </section>

            <section aria-labelledby='finance-methods-heading'>
              <h3
                id='finance-methods-heading'
                className='text-sm font-semibold'
              >
                {t('Payment methods')}
              </h3>
              <p className='text-muted-foreground mt-1 text-xs'>
                {t('Include revenue')}
              </p>
              <div className='mt-2 divide-y'>
                {(overview?.payment_methods ?? []).map((method) => (
                  <PaymentMethodRow
                    key={method.method}
                    method={method}
                    summary={paymentMethodSummary(
                      method.method,
                      overview?.revenue_by_method,
                      overview?.refund_by_method
                    )}
                    currency={overview?.currency ?? 'USD'}
                    onChange={(value) => void updateMethod(method, value)}
                    onView={() => {
                      openLedger(method.method, method.label || method.method)
                      void navigate({
                        to: '/finance',
                        search: {
                          user_id: undefined,
                          payment_method: method.method,
                        },
                      })
                    }}
                    viewing={selectedPaymentMethod === method.method}
                  />
                ))}
                {(overview?.payment_methods ?? []).length === 0 ? (
                  <p className='text-muted-foreground py-4 text-sm'>
                    {t('No data')}
                  </p>
                ) : null}
              </div>
            </section>
          </div>

          <Separator />

          <div className='grid gap-8 lg:grid-cols-[minmax(0,1fr)_20rem]'>
            <section aria-labelledby='finance-users-heading'>
              <div className='flex items-baseline justify-between gap-3'>
                <div>
                  <h3
                    id='finance-users-heading'
                    className='text-sm font-semibold'
                  >
                    {t('User spending')}
                  </h3>
                  <p className='text-muted-foreground mt-1 text-xs'>
                    {t('View user')}
                  </p>
                </div>
                <span className='text-muted-foreground text-xs'>
                  {t('Requests')}: {compact(overview?.tokens.requests ?? 0)}
                </span>
              </div>
              <div className='mt-2 divide-y'>
                {(overview?.users ?? []).slice(0, 10).map((user) => (
                  <UserRow
                    key={user.user_id}
                    user={user}
                    days={days}
                    paymentMethod={selectedPaymentMethod}
                  />
                ))}
                {(overview?.users ?? []).length === 0 ? (
                  <p className='text-muted-foreground py-4 text-sm'>
                    {t('No data')}
                  </p>
                ) : null}
              </div>
            </section>

            <section aria-labelledby='finance-expense-heading'>
              <div className='flex items-center justify-between gap-3'>
                <h3
                  id='finance-expense-heading'
                  className='text-sm font-semibold'
                >
                  {t('External expense')}
                </h3>
                <Button
                  variant='ghost'
                  size='sm'
                  onClick={() => setExpenseOpen((open) => !open)}
                >
                  {t('Add expense')}
                </Button>
              </div>
              {expenseOpen ? (
                <div className='mt-4 space-y-3'>
                  <Input
                    inputMode='decimal'
                    placeholder='0.00'
                    value={expenseAmount}
                    onChange={(event) => setExpenseAmount(event.target.value)}
                    aria-label={t('Amount')}
                  />
                  <Input
                    placeholder={t('Category')}
                    value={expenseCategory}
                    onChange={(event) => setExpenseCategory(event.target.value)}
                    aria-label={t('Category')}
                  />
                  <Input
                    placeholder={t('Note')}
                    value={expenseNote}
                    onChange={(event) => setExpenseNote(event.target.value)}
                    aria-label={t('Note')}
                  />
                  <Button
                    className='w-full'
                    disabled={
                      expenseMutation.isPending || !(Number(expenseAmount) > 0)
                    }
                    onClick={() => void expenseMutation.mutateAsync()}
                  >
                    {t('Record expense')}
                  </Button>
                </div>
              ) : (
                <p className='text-muted-foreground mt-2 text-xs'>
                  {t('Append-only ledger')}
                </p>
              )}
            </section>
          </div>

          {search.user_id ? (
            <UserDetail
              userId={search.user_id}
              days={days}
              paymentMethod={selectedPaymentMethod}
              onViewLedger={() =>
                openLedger(
                  selectedPaymentMethod,
                  selectedPaymentMethodLabel,
                  search.user_id
                )
              }
            />
          ) : null}
          <LedgerEntriesDialog
            open={ledgerOpen}
            onOpenChange={setLedgerOpen}
            days={days}
            paymentMethod={ledgerPaymentMethod}
            paymentMethodLabel={ledgerPaymentMethodLabel}
            initialUserID={ledgerUserID}
          />
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
