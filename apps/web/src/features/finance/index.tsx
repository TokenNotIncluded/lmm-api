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
import { getRouteApi, Link, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, RefreshCw, WalletCards } from 'lucide-react'
import { useMemo, useState, type ReactNode } from 'react'
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

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'

import {
  createFinanceExpense,
  getFinanceOverview,
  getFinanceUser,
  updateFinancePaymentMethod,
  type FinancePaymentMethod,
  type FinanceUserMetric,
} from './api'

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
  onChange,
}: {
  method: FinancePaymentMethod
  onChange: (value: Partial<FinancePaymentMethod>) => void
}) {
  const { t } = useTranslation()
  return (
    <div className='flex items-center gap-3 py-3'>
      <WalletCards
        className='text-muted-foreground size-4 shrink-0'
        aria-hidden='true'
      />
      <div className='min-w-0 flex-1'>
        <p className='truncate text-sm font-medium'>
          {method.label || method.method}
        </p>
        <p className='text-muted-foreground truncate text-xs'>
          {method.method}
        </p>
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

function UserRow({ user, days }: { user: FinanceUserMetric; days: number }) {
  const { t } = useTranslation()
  return (
    <Link
      to='/finance'
      search={{ user_id: user.user_id }}
      className='group flex items-center gap-3 py-3'
    >
      <span className='bg-muted text-muted-foreground flex size-8 shrink-0 items-center justify-center rounded-full text-xs'>
        {String(user.user_id).slice(-2)}
      </span>
      <span className='min-w-0 flex-1'>
        <span className='block truncate text-sm font-medium'>
          #{user.user_id}
        </span>
        <span className='text-muted-foreground block text-xs'>
          {compact(user.token_units)} tokens · {user.requests}{' '}
          {t('Requests').toLowerCase()}
        </span>
      </span>
      <span className='text-right'>
        <span className='block text-sm font-medium'>
          {money(user.expense_micros)}
        </span>
        <span className='text-muted-foreground block text-xs'>{days}d</span>
      </span>
    </Link>
  )
}

function UserDetail({ userId, days }: { userId: number; days: number }) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const query = useQuery({
    queryKey: ['finance-user', userId, days],
    queryFn: () => getFinanceUser(userId, days),
  })
  const overview = query.data?.data
  let detailContent: ReactNode
  if (query.isLoading) {
    detailContent = (
      <p className='text-muted-foreground mt-5 text-sm'>{t('Loading...')}</p>
    )
  } else if (overview) {
    detailContent = (
      <div className='mt-5 grid gap-4 sm:grid-cols-3'>
        <Metric label={t('Expenses')} value={money(overview.expense_micros)} />
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
      <div className='flex items-center justify-between gap-3'>
        <div>
          <p className='text-muted-foreground text-xs'>{t('User spending')}</p>
          <h3 id='finance-user-detail' className='mt-1 text-lg font-semibold'>
            #{userId}
          </h3>
        </div>
        <Button
          variant='ghost'
          size='sm'
          onClick={() => void navigate({ to: '/finance' })}
        >
          <ArrowLeft data-icon='inline-start' aria-hidden='true' />
          {t('Back')}
        </Button>
      </div>
      {detailContent}
    </section>
  )
}

export function Finance() {
  const { t } = useTranslation()
  const search = route.useSearch()
  const queryClient = useQueryClient()
  const [days, setDays] = useState<(typeof WINDOWS)[number]>(30)
  const [paymentMethod, setPaymentMethod] = useState('')
  const [expenseOpen, setExpenseOpen] = useState(false)
  const [expenseAmount, setExpenseAmount] = useState('')
  const [expenseCategory, setExpenseCategory] = useState('')
  const [expenseNote, setExpenseNote] = useState('')
  const overviewQuery = useQuery({
    queryKey: ['finance-overview', days, paymentMethod],
    queryFn: () => getFinanceOverview(days, paymentMethod || undefined),
  })
  const overview = overviewQuery.data?.data
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
        expense: item.expense_micros / 1_000_000,
      })),
    [overview?.daily]
  )

  const updateMethod = async (
    method: FinancePaymentMethod,
    value: Partial<FinancePaymentMethod>
  ) => {
    await updateFinancePaymentMethod(method.method, value)
    await queryClient.invalidateQueries({ queryKey: ['finance-overview'] })
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
              <select
                value={paymentMethod}
                onChange={(event) => setPaymentMethod(event.target.value)}
                className='border-input bg-background text-foreground h-8 rounded-md border px-2 text-xs'
                aria-label={t('Payment method')}
              >
                <option value=''>
                  {t('Payment method')} · {t('All')}
                </option>
                {(overview?.payment_methods ?? []).map((method) => (
                  <option key={method.method} value={method.method}>
                    {method.label || method.method}
                  </option>
                ))}
              </select>
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
              value={money(overview?.revenue_micros ?? 0)}
            />
            <Metric
              label={t('Expenses')}
              value={money(overview?.expense_micros ?? 0)}
            />
            <Metric
              label={t('Profit')}
              value={money(overview?.profit_micros ?? 0)}
              tone={
                (overview?.profit_micros ?? 0) >= 0 ? 'positive' : 'negative'
              }
              detail={`${t('Profit margin')}: ${overview?.revenue_micros ? Math.round((overview.profit_micros / overview.revenue_micros) * 100 * 10) / 10 : 0}%`}
            />
            <Metric
              label={t('Token economy')}
              value={compact(overview?.tokens.total_tokens ?? 0)}
              detail={`${compact(overview?.tokens.requests ?? 0)} ${t('Requests').toLowerCase()}`}
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
                          stopColor='hsl(var(--chart-1))'
                          stopOpacity={0.24}
                        />
                        <stop
                          offset='95%'
                          stopColor='hsl(var(--chart-1))'
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
                          stopColor='hsl(var(--chart-2))'
                          stopOpacity={0.18}
                        />
                        <stop
                          offset='95%'
                          stopColor='hsl(var(--chart-2))'
                          stopOpacity={0}
                        />
                      </linearGradient>
                    </defs>
                    <CartesianGrid
                      vertical={false}
                      stroke='hsl(var(--border) / .45)'
                    />
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
                        name === 'revenue' ? t('Revenue') : t('Expenses'),
                      ]}
                    />
                    <Area
                      type='monotone'
                      dataKey='revenue'
                      stroke='hsl(var(--chart-1))'
                      fill='url(#finance-revenue)'
                      strokeWidth={2}
                    />
                    <Area
                      type='monotone'
                      dataKey='expense'
                      stroke='hsl(var(--chart-2))'
                      fill='url(#finance-expense)'
                      strokeWidth={2}
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
                    onChange={(value) => void updateMethod(method, value)}
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
                  <UserRow key={user.user_id} user={user} days={days} />
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
            <UserDetail userId={search.user_id} days={days} />
          ) : null}
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
