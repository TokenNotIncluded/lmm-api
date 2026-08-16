/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { formatTimestampRelative, formatTimestampToDate } from '@/lib/format'
import { ROLE } from '@/lib/roles'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

import {
  getTodos,
  markAllTodosRead,
  markTodoRead,
  type TodoCategory,
  type TodoItem,
} from './api'
import { todoItemTitleKey } from './todo-labels'
import { todoItemHasDestination } from './todo-navigation'

const CATEGORY_LABELS: Record<TodoCategory, string> = {
  all: 'All',
  open_source_bounty_review: 'Challenge reviews',
  open_source_bounty: 'Bounty notifications',
  developer_access: 'Developer access',
  account_action: 'Account actions',
  security_incident: 'Security incidents',
  security_review: 'Security reviews',
}

function detailString(item: TodoItem, key: string) {
  const value = item.details?.[key]
  return typeof value === 'string' ? value : ''
}

function detailNumber(item: TodoItem, key: string) {
  const value = item.details?.[key]
  return typeof value === 'number' ? value : undefined
}

export function UnifiedTodoList() {
  const { t, i18n } = useTranslation()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const user = useAuthStore((state) => state.auth.user)
  const [category, setCategory] = useState<TodoCategory>('all')
  const query = useQuery({
    queryKey: ['todos', category],
    queryFn: () => getTodos(category),
    staleTime: 10_000,
  })
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: ['todos'], exact: false })
  const markOne = useMutation({ mutationFn: markTodoRead, onSuccess: refresh })
  const markAll = useMutation({
    mutationFn: markAllTodosRead,
    onSuccess: refresh,
  })

  const openItem = async (item: TodoItem) => {
    if (!item.read) await markOne.mutateAsync(item)
    const projectId = detailNumber(item, 'project_id')
    if (projectId) {
      await navigate({
        to: '/challenges/$challengeId',
        params: { challengeId: String(projectId) },
      })
      return
    }
    if (item.category === 'security_review') {
      await navigate({
        to: '/system-settings/security/$section',
        params: { section: 'advanced-security' },
      })
      return
    }
    if (
      item.category === 'security_incident' ||
      item.category === 'developer_access'
    ) {
      const username = detailString(item, 'username')
      if (username) {
        await navigate({
          to: '/users',
          search: {
            page: 1,
            pageSize: undefined,
            filter: username,
            status: [],
            role: [],
            group: '',
            l0Only: false,
          },
        })
      }
    }
  }

  const categories = query.data?.categories ?? []
  const visibleCategories: TodoCategory[] = [
    'all',
    ...categories
      .filter(
        (item) =>
          item.total > 0 ||
          item.key === 'open_source_bounty_review' ||
          ((user?.role ?? 0) >= ROLE.ADMIN &&
            item.key === 'security_incident') ||
          ((user?.role ?? 0) >= ROLE.ADMIN && item.key === 'security_review') ||
          ((user?.role ?? 0) >= ROLE.ADMIN &&
            (item.key === 'developer_access' || item.key === 'account_action'))
      )
      .map((item) => item.key),
  ]

  return (
    <section aria-busy={query.isLoading}>
      <div className='border-border flex flex-wrap items-center gap-x-8 gap-y-3 border-b pb-4'>
        {visibleCategories.map((key) => {
          const summary = categories.find((item) => item.key === key)
          const unread =
            key === 'all' ? query.data?.total_unread_count : summary?.unread
          return (
            <button
              key={key}
              type='button'
              className={cn(
                'text-muted-foreground hover:text-foreground py-1 text-sm transition-colors',
                category === key && 'text-foreground font-medium'
              )}
              onClick={() => setCategory(key)}
            >
              {t(CATEGORY_LABELS[key])}
              {unread ? ` ${unread}` : ''}
            </button>
          )
        })}
        {(query.data?.total_unread_count ?? 0) > 0 ? (
          <Button
            type='button'
            variant='link'
            size='sm'
            className='ml-auto h-auto px-0'
            disabled={markAll.isPending}
            onClick={() => markAll.mutate()}
          >
            {t('Mark all as read')}
          </Button>
        ) : null}
      </div>

      {query.isError ? (
        <div className='py-12 text-sm'>
          <p>{t('Failed to load to-dos')}</p>
          <Button
            variant='link'
            className='mt-2 h-auto px-0'
            onClick={() => query.refetch()}
          >
            {t('Retry')}
          </Button>
        </div>
      ) : query.isLoading ? (
        <p className='text-muted-foreground py-12 text-sm'>{t('Loading')}</p>
      ) : query.data?.items.length ? (
        <div>
          {query.data.items.map((item) => {
            let participant = detailString(item, 'participant_username')
            if (
              !participant &&
              (item.category === 'security_incident' ||
                item.category === 'developer_access')
            ) {
              participant = detailString(item, 'username')
            }
            const applicantId = detailNumber(item, 'user_id')
            const applicantEmail = detailString(item, 'email')
            const title = t(todoItemTitleKey(item.title))
            const canOpen = todoItemHasDestination(item)
            return (
              <button
                key={item.id}
                type='button'
                className='border-border hover:text-foreground flex w-full items-start gap-4 border-b py-7 text-left transition-colors sm:items-center'
                onClick={() => void openItem(item)}
                disabled={markOne.isPending && !item.read}
              >
                <span
                  className={cn(
                    'bg-foreground size-1.5 shrink-0 rounded-full',
                    item.read && 'opacity-0'
                  )}
                  aria-hidden='true'
                />
                <span className='min-w-0 flex-1'>
                  <span className='flex flex-wrap items-baseline gap-x-2 gap-y-1'>
                    <span className='font-medium'>{title}</span>
                    {participant ? (
                      <span className='text-muted-foreground text-sm'>
                        @{participant}
                      </span>
                    ) : null}
                    {applicantId ? (
                      <span className='text-muted-foreground text-xs'>
                        {t('User ID')} {applicantId}
                      </span>
                    ) : null}
                  </span>
                  {applicantEmail ? (
                    <span className='text-muted-foreground mt-0.5 block truncate text-xs'>
                      {applicantEmail}
                    </span>
                  ) : null}
                  <span className='text-muted-foreground mt-1 block truncate text-sm'>
                    {item.summary}
                  </span>
                </span>
                <time
                  className='text-muted-foreground shrink-0 pt-0.5 text-xs sm:pt-0'
                  dateTime={new Date(item.updated_at * 1000).toISOString()}
                  title={formatTimestampToDate(item.updated_at)}
                >
                  {formatTimestampRelative(
                    item.updated_at,
                    'seconds',
                    i18n.language
                  )}
                </time>
                {canOpen ? (
                  <ChevronRight
                    className='size-4 shrink-0'
                    aria-hidden='true'
                  />
                ) : null}
              </button>
            )
          })}
        </div>
      ) : (
        <div className='py-16 text-center'>
          <p className='font-medium'>{t('No pending to-dos')}</p>
          <p className='text-muted-foreground mt-2 text-sm'>
            {t(
              'Submitted challenge work and account requests will appear here.'
            )}
          </p>
        </div>
      )}
    </section>
  )
}
