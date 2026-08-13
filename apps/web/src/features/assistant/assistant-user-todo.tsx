/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import {
  Alert02Icon,
  ArrowRight01Icon,
  CheckmarkCircle02Icon,
  Clock01Icon,
  CustomerSupportIcon,
  ReloadIcon,
  ShieldKeyIcon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

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
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'

import {
  getAssistantHandoff as getLatestAssistantHandoff,
  type AssistantHandoff,
} from './api'
import { redactAssistantMessageForDisplay } from './assistant-message-safety'

export type AssistantUserTodoProps = {
  /** Open the assistant and let the user continue the conversation. */
  onContinueWithAi?: () => void
  /** Navigate to the page related to the request (for example, API keys). */
  onViewRelatedPage?: () => void
  className?: string
}

const assistantUserTodoQueryKey = ['assistant-handoff'] as const

function PrivacyNotice() {
  const { t } = useTranslation()

  return (
    <Alert
      className='min-w-0 overflow-hidden [&_[data-slot=alert-description]]:[overflow-wrap:anywhere]'
      data-testid='assistant-user-todo-privacy'
    >
      <HugeiconsIcon icon={ShieldKeyIcon} strokeWidth={2} aria-hidden='true' />
      <AlertTitle className='min-w-0 break-words'>
        {t('Privacy and redaction')}
      </AlertTitle>
      <AlertDescription className='min-w-0 break-words'>
        {t(
          'Only you and authorized administrators can view this request. Obvious secrets are redacted before storage and display.'
        )}
      </AlertDescription>
    </Alert>
  )
}

function TodoActions(props: {
  onContinueWithAi?: () => void
  onViewRelatedPage?: () => void
  onRefresh: () => void
  refreshing: boolean
}) {
  const { t } = useTranslation()

  return (
    <div
      className='flex min-w-0 flex-col gap-2 sm:flex-row'
      data-testid='assistant-user-todo-actions'
    >
      <Button
        type='button'
        variant='default'
        className='w-full min-w-0 sm:flex-1'
        data-testid='assistant-user-todo-continue-ai'
        onClick={props.onContinueWithAi}
      >
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          data-icon='inline-start'
          aria-hidden='true'
        />
        <span className='min-w-0 overflow-hidden text-ellipsis'>
          {t('Continue with AI')}
        </span>
      </Button>
      <Button
        type='button'
        variant='outline'
        className='w-full min-w-0 sm:flex-1'
        data-testid='assistant-user-todo-related'
        onClick={props.onViewRelatedPage}
      >
        <HugeiconsIcon
          icon={CustomerSupportIcon}
          strokeWidth={2}
          data-icon='inline-start'
          aria-hidden='true'
        />
        <span className='min-w-0 overflow-hidden text-ellipsis'>
          {t('View related page')}
        </span>
      </Button>
      <Button
        type='button'
        variant='ghost'
        className='w-full sm:w-auto'
        data-testid='assistant-user-todo-refresh'
        aria-label={t('Refresh')}
        title={t('Refresh')}
        onClick={props.onRefresh}
        disabled={props.refreshing}
      >
        <HugeiconsIcon
          icon={ReloadIcon}
          strokeWidth={2}
          data-icon='inline-start'
          aria-hidden='true'
        />
        {t('Refresh')}
      </Button>
    </div>
  )
}

function MessageBlock(props: {
  label: string
  message: string
  className?: string
}) {
  const safeMessage = redactAssistantMessageForDisplay(
    props.message,
    '[REDACTED]'
  )

  return (
    <div
      className={cn(
        'grid min-w-0 gap-1.5 rounded-lg border p-3',
        props.className
      )}
    >
      <p className='text-muted-foreground text-xs font-medium'>{props.label}</p>
      {safeMessage.content ? (
        <p className='min-w-0 text-sm leading-6 [overflow-wrap:anywhere] break-words whitespace-pre-wrap'>
          {safeMessage.content}
        </p>
      ) : null}
      {safeMessage.redacted ? (
        <p className='text-muted-foreground min-w-0 text-xs leading-5 [overflow-wrap:anywhere]'>
          {`[REDACTED] ${props.label}`}
        </p>
      ) : null}
    </div>
  )
}

function PendingTodo(props: { handoff: AssistantHandoff }) {
  const { t } = useTranslation()

  return (
    <div
      className='grid min-w-0 gap-3'
      data-testid='assistant-user-todo-pending'
    >
      <div className='flex min-w-0 flex-wrap items-center justify-between gap-2'>
        <div className='flex min-w-0 items-center gap-2'>
          <HugeiconsIcon
            icon={Clock01Icon}
            className='text-warning size-4 shrink-0'
            strokeWidth={2}
            aria-hidden='true'
          />
          <p className='min-w-0 text-sm font-medium break-words'>
            {t('Waiting for an administrator')}
          </p>
        </div>
        <Badge variant='warning'>{t('Pending')}</Badge>
      </div>
      <p className='text-muted-foreground min-w-0 text-sm leading-6 break-words'>
        {t(
          'Your request is in the administrator queue. We will show the reply here when it is resolved.'
        )}
      </p>
      <MessageBlock label={t('Your request')} message={props.handoff.message} />
    </div>
  )
}

function ResolvedTodo(props: { handoff: AssistantHandoff }) {
  const { t } = useTranslation()
  const adminNote = props.handoff.admin_note.trim()

  return (
    <div
      className='grid min-w-0 gap-3'
      data-testid='assistant-user-todo-resolved'
    >
      <div className='flex min-w-0 flex-wrap items-center justify-between gap-2'>
        <div className='flex min-w-0 items-center gap-2'>
          <HugeiconsIcon
            icon={CheckmarkCircle02Icon}
            className='text-success size-4 shrink-0'
            strokeWidth={2}
            aria-hidden='true'
          />
          <p className='min-w-0 text-sm font-medium break-words'>
            {t('Administrator replied')}
          </p>
        </div>
        <Badge variant='secondary'>{t('Resolved')}</Badge>
      </div>
      <MessageBlock label={t('Your request')} message={props.handoff.message} />
      <MessageBlock
        className='bg-muted/30'
        label={t('Administrator reply')}
        message={
          adminNote || t('The administrator marked this request resolved.')
        }
      />
    </div>
  )
}

function LoadingTodo() {
  const { t } = useTranslation()

  return (
    <div
      className='grid min-w-0 gap-3'
      data-testid='assistant-user-todo-loading'
      aria-label={t('Loading personal tasks...')}
    >
      <div className='flex items-center justify-between gap-3'>
        <Skeleton className='h-4 w-40 max-w-full' />
        <Skeleton className='h-5 w-16 shrink-0' />
      </div>
      <Skeleton className='h-20 w-full max-w-full' />
      <Skeleton className='h-9 w-full max-w-full' />
    </div>
  )
}

function EmptyTodo() {
  const { t } = useTranslation()

  return (
    <Empty
      className='min-w-0 border-dashed px-3 py-8'
      data-testid='assistant-user-todo-empty'
    >
      <EmptyHeader className='min-w-0'>
        <EmptyMedia variant='icon'>
          <HugeiconsIcon
            icon={CustomerSupportIcon}
            strokeWidth={2}
            aria-hidden='true'
          />
        </EmptyMedia>
        <EmptyTitle>{t('No support requests yet')}</EmptyTitle>
        <EmptyDescription className='min-w-0 [overflow-wrap:anywhere] break-words'>
          {t(
            'When you ask an administrator for help, your request and its status will appear here.'
          )}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  )
}

export function AssistantUserTodo(props: AssistantUserTodoProps) {
  const { t } = useTranslation()
  const handoffQuery = useQuery({
    queryKey: assistantUserTodoQueryKey,
    queryFn: getLatestAssistantHandoff,
    staleTime: 30_000,
    retry: false,
  })

  const refresh = () => {
    void handoffQuery.refetch()
  }

  const handoff = handoffQuery.data
  const hasResolvedHandoff = handoff?.status === 'resolved'
  const hasPendingHandoff = handoff?.status === 'pending'

  return (
    <section
      className={cn(
        'w-full min-w-0 max-w-full overflow-hidden',
        props.className
      )}
      data-testid='assistant-user-todo'
      aria-labelledby='assistant-user-todo-title'
    >
      <Card className='max-w-full min-w-0'>
        <CardHeader className='min-w-0'>
          <CardTitle
            id='assistant-user-todo-title'
            className='min-w-0 break-words'
          >
            {t('Your support tasks')}
          </CardTitle>
          <CardDescription className='min-w-0 break-words'>
            {t('Track your request to the administrator and its next step.')}
          </CardDescription>
        </CardHeader>
        <CardContent className='grid min-w-0 gap-4'>
          <PrivacyNotice />
          {handoffQuery.isPending ? <LoadingTodo /> : null}
          {handoffQuery.isError ? (
            <Alert
              className='min-w-0 overflow-hidden'
              data-testid='assistant-user-todo-error'
              variant='destructive'
            >
              <HugeiconsIcon
                icon={Alert02Icon}
                strokeWidth={2}
                aria-hidden='true'
              />
              <AlertTitle className='min-w-0 break-words'>
                {t('Unable to load your support tasks')}
              </AlertTitle>
              <AlertDescription className='min-w-0 [overflow-wrap:anywhere] break-words'>
                {t('Refresh to try loading your personal request again.')}
              </AlertDescription>
              <AlertAction>
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  onClick={refresh}
                  disabled={handoffQuery.isFetching}
                >
                  {t('Refresh')}
                </Button>
              </AlertAction>
            </Alert>
          ) : null}
          {!handoffQuery.isPending && !handoffQuery.isError ? (
            hasPendingHandoff ? (
              <PendingTodo handoff={handoff} />
            ) : hasResolvedHandoff ? (
              <ResolvedTodo handoff={handoff} />
            ) : (
              <EmptyTodo />
            )
          ) : null}
          <TodoActions
            onContinueWithAi={props.onContinueWithAi}
            onViewRelatedPage={props.onViewRelatedPage}
            onRefresh={refresh}
            refreshing={handoffQuery.isFetching}
          />
        </CardContent>
      </Card>
    </section>
  )
}
