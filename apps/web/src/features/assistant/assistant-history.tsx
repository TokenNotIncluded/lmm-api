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
  Alert02Icon,
  Archive01Icon,
  ArchiveRestoreIcon,
  ShieldKeyIcon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Fragment, useMemo, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Response } from '@/components/ai-elements/response'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { toIntlLocale } from '@/i18n/languages'
import { ROLE } from '@/lib/roles'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

import {
  archiveAssistantConversation,
  getAssistantConversationHistory,
  getAssistantConversationHistoryDetail,
  unarchiveAssistantConversation,
  type AssistantConversationHistoryDetail,
  type AssistantConversationHistoryItem,
  type AssistantConversationHistoryMessage,
} from './api'
import { redactAssistantMessageForDisplay } from './assistant-message-safety'

function assistantHistoryErrorStatus(error: unknown): number | null {
  const status = (error as { response?: { status?: unknown } } | null)?.response
    ?.status
  return typeof status === 'number' ? status : null
}

function HistoryMessage(props: {
  message: AssistantConversationHistoryMessage
}) {
  const { t } = useTranslation()
  const safeMessage = redactAssistantMessageForDisplay(
    props.message.content,
    t(
      'Sensitive content is hidden and can only be accessed from a private card.'
    )
  )
  return (
    <div className='grid gap-1 py-2'>
      <p className='text-muted-foreground text-[11px] font-medium'>
        {props.message.role === 'assistant' ? t('Service guide') : t('You')}
      </p>
      {props.message.role === 'assistant' && safeMessage.content ? (
        <Response className='text-sm leading-6' final>
          {safeMessage.content}
        </Response>
      ) : props.message.role !== 'secure_card' && safeMessage.content ? (
        <p className='text-sm leading-6 whitespace-pre-wrap'>
          {safeMessage.content}
        </p>
      ) : null}
      {props.message.cards?.length || safeMessage.redacted ? (
        <div className='text-success flex items-center gap-1.5 text-xs leading-5'>
          <HugeiconsIcon
            icon={ShieldKeyIcon}
            className='size-3.5 shrink-0'
            strokeWidth={2}
            aria-hidden='true'
          />
          {props.message.cards
            ?.map((card) => card.label)
            .filter(Boolean)
            .join('、') ||
            t(
              'Sensitive content is hidden and can only be accessed from a private card.'
            )}
        </div>
      ) : null}
    </div>
  )
}

export function AssistantHistory(props: {
  active: boolean
  onOpenConversation: (conversation: AssistantConversationHistoryItem) => void
  ownerUser?: { id: number; username: string }
  presentation?: 'cards' | 'rows'
  limit?: number
}) {
  const { t, i18n } = useTranslation()
  const queryClient = useQueryClient()
  const authUser = useAuthStore((state) => state.auth.user)
  const canAudit = authUser?.role !== undefined && authUser.role >= ROLE.ADMIN
  const [scope, setScope] = useState<'self' | 'audit'>('self')
  const [auditUserIdInput, setAuditUserIdInput] = useState('')
  const [auditUserId, setAuditUserId] = useState<number | null>(null)
  const [auditInputError, setAuditInputError] = useState(false)
  const [filter, setFilter] = useState<'active' | 'archived'>('active')
  const showingArchived = filter === 'archived'
  const fixedScope = props.ownerUser
    ? props.ownerUser.id === authUser?.id
      ? 'self'
      : 'audit'
    : null
  const effectiveScope = fixedScope ?? (canAudit ? scope : 'self')
  const activeUserId =
    effectiveScope === 'audit'
      ? (props.ownerUser?.id ?? auditUserId ?? undefined)
      : undefined
  const historyLimit = props.limit
  const historyQuery = useQuery({
    queryKey: [
      'assistant-conversations',
      effectiveScope,
      activeUserId ?? null,
      filter,
      ...(historyLimit === undefined ? [] : [historyLimit]),
    ],
    queryFn: () =>
      getAssistantConversationHistory(
        showingArchived,
        activeUserId,
        historyLimit
      ),
    enabled:
      props.active && (effectiveScope === 'self' || activeUserId !== undefined),
    staleTime: 30_000,
    retry: false,
  })
  const archiveMutation = useMutation({
    mutationFn: ({ id, archived }: { id: number; archived: boolean }) =>
      archived
        ? unarchiveAssistantConversation(id)
        : archiveAssistantConversation(id),
    onSuccess: async (_result, variables) => {
      await queryClient.invalidateQueries({
        queryKey: ['assistant-conversations'],
      })
      toast.success(
        t(
          variables.archived
            ? 'Conversation restored.'
            : 'Conversation archived.'
        )
      )
    },
    onError: (_error, variables) => {
      toast.error(
        t(
          variables.archived
            ? 'Unable to restore conversation. Try again.'
            : 'Unable to archive conversation. Try again.'
        )
      )
    },
  })
  const dateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(toIntlLocale(i18n.language), {
        dateStyle: 'medium',
        timeStyle: 'short',
      }),
    [i18n.language]
  )
  const conversations = historyQuery.data?.conversations ?? []
  const status = assistantHistoryErrorStatus(historyQuery.error)

  const selectSelfScope = () => {
    setScope('self')
    setAuditUserId(null)
    setAuditInputError(false)
  }

  const selectAuditScope = () => {
    setScope('audit')
    setAuditInputError(false)
  }

  const submitAuditUserId = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const input = event.currentTarget.elements.namedItem(
      'assistant-history-audit-user-id'
    ) as HTMLInputElement | null
    const value = (input?.value ?? auditUserIdInput).trim()
    if (!/^[1-9]\d*$/.test(value)) {
      setAuditInputError(true)
      return
    }
    const nextUserId = Number(value)
    if (!Number.isSafeInteger(nextUserId) || nextUserId <= 0) {
      setAuditInputError(true)
      return
    }
    setAuditInputError(false)
    setAuditUserId(nextUserId)
    setScope('audit')
  }

  const historyErrorDescription =
    status === 400
      ? t('Enter a positive integer')
      : status === 403
        ? t('Conversation history is not available to this account.')
        : status === 404
          ? t('This conversation no longer exists or is unavailable.')
          : t('Unable to load conversation history. Try again.')

  return (
    <div className='grid gap-3'>
      {canAudit && !props.ownerUser ? (
        <div className='grid gap-3 rounded-lg border p-3'>
          <div className='flex flex-wrap gap-2'>
            <Button
              type='button'
              variant={effectiveScope === 'self' ? 'secondary' : 'outline'}
              size='sm'
              aria-pressed={effectiveScope === 'self'}
              onClick={selectSelfScope}
            >
              {t('My conversations')}
            </Button>
            <Button
              type='button'
              variant={effectiveScope === 'audit' ? 'secondary' : 'outline'}
              size='sm'
              aria-pressed={effectiveScope === 'audit'}
              onClick={selectAuditScope}
            >
              {t('User audit')}
            </Button>
          </div>
          {effectiveScope === 'audit' ? (
            <form className='grid gap-2' onSubmit={submitAuditUserId}>
              <Label htmlFor='assistant-history-audit-user-id'>
                {t('User ID')}
              </Label>
              <div className='flex flex-wrap gap-2 sm:flex-nowrap'>
                <Input
                  id='assistant-history-audit-user-id'
                  value={auditUserIdInput}
                  onChange={(event) => {
                    setAuditUserIdInput(event.target.value)
                    setAuditUserId(null)
                    setAuditInputError(false)
                  }}
                  inputMode='numeric'
                  autoComplete='off'
                  placeholder={t('Enter a positive integer')}
                  aria-invalid={auditInputError}
                />
                <Button type='submit' variant='outline' className='shrink-0'>
                  {t('View')}
                </Button>
              </div>
              {auditInputError ? (
                <p className='text-destructive text-xs' role='alert'>
                  {t('Enter a positive integer')}
                </p>
              ) : null}
            </form>
          ) : null}
        </div>
      ) : null}
      {effectiveScope === 'audit' && activeUserId !== undefined ? (
        <div className='grid gap-1 rounded-lg border p-3'>
          <p className='text-sm font-medium'>{t('User audit')}</p>
          <p className='text-muted-foreground text-xs leading-5'>
            {props.ownerUser?.username
              ? `${props.ownerUser.username} · `
              : `${t('Lower-access user conversation')} · `}
            {t('User ID')}: {activeUserId}
          </p>
        </div>
      ) : null}
      <div className='flex flex-wrap gap-2'>
        <Button
          type='button'
          variant={showingArchived ? 'outline' : 'secondary'}
          size='sm'
          aria-pressed={!showingArchived}
          onClick={() => setFilter('active')}
        >
          {t('Active conversations')}
        </Button>
        <Button
          type='button'
          variant={showingArchived ? 'secondary' : 'outline'}
          size='sm'
          aria-pressed={showingArchived}
          onClick={() => setFilter('archived')}
        >
          {t('Archived conversations')}
        </Button>
      </div>
      {historyQuery.isLoading ? (
        <div
          className='grid gap-3'
          aria-label={t('Loading conversation history...')}
        >
          <Skeleton className='h-16 w-full' />
          <Skeleton className='h-16 w-full' />
        </div>
      ) : historyQuery.isError ? (
        <Alert variant='destructive'>
          <HugeiconsIcon
            icon={Alert02Icon}
            strokeWidth={2}
            aria-hidden='true'
          />
          <AlertTitle>{t('Conversation history')}</AlertTitle>
          <AlertDescription>{historyErrorDescription}</AlertDescription>
        </Alert>
      ) : effectiveScope === 'audit' && activeUserId === undefined ? (
        <p className='text-muted-foreground py-8 text-center text-sm leading-6'>
          {t('Enter a positive integer')}
        </p>
      ) : conversations.length === 0 ? (
        <p className='text-muted-foreground py-8 text-center text-sm leading-6'>
          {effectiveScope === 'audit'
            ? `${t('No visible conversation history yet.')} · ${t('User ID')}: ${activeUserId}`
            : t(
                showingArchived
                  ? 'No archived conversations yet.'
                  : 'No active conversations yet.'
              )}
        </p>
      ) : (
        <div
          data-presentation={props.presentation ?? 'cards'}
          data-testid='assistant-history-list'
        >
          {conversations.map((conversation, index) => {
            const safePreview = redactAssistantMessageForDisplay(
              conversation.last_message_preview,
              t(
                'Sensitive content is hidden and can only be accessed from a private card.'
              )
            ).content
            const canManage =
              effectiveScope === 'self' && conversation.owner === 'self'
            const actionPending =
              archiveMutation.isPending &&
              archiveMutation.variables?.id === conversation.id
            return (
              <Fragment key={conversation.id}>
                {props.presentation === 'rows' && index > 0 ? (
                  <Separator />
                ) : null}
                <article
                  className={cn(
                    'grid min-w-0 gap-2',
                    props.presentation === 'rows'
                      ? 'py-4'
                      : 'mb-3 rounded-lg border p-3 last:mb-0'
                  )}
                  data-testid='assistant-history-item'
                >
                  <div className='flex min-w-0 items-start justify-between gap-3'>
                    <div className='min-w-0'>
                      <p className='line-clamp-2 text-sm font-medium'>
                        <span className='sr-only'>
                          {conversation.owner === 'self'
                            ? `${t('Your conversation')}: `
                            : `${t('Lower-access user conversation')}: `}
                        </span>
                        {conversation.title}
                      </p>
                      <p className='text-muted-foreground mt-0.5 text-xs'>
                        {dateFormatter.format(
                          new Date(conversation.updated_at * 1000)
                        )}
                      </p>
                    </div>
                    <div className='flex shrink-0 flex-wrap justify-end gap-2'>
                      <Button
                        type='button'
                        variant={
                          props.presentation === 'rows' ? 'ghost' : 'outline'
                        }
                        size='sm'
                        onClick={() => props.onOpenConversation(conversation)}
                      >
                        {t('View')}
                      </Button>
                      {canManage ? (
                        <Button
                          type='button'
                          variant='ghost'
                          size='sm'
                          aria-label={t(
                            showingArchived
                              ? 'Restore conversation'
                              : 'Archive conversation'
                          )}
                          disabled={actionPending}
                          onClick={(event) => {
                            event.stopPropagation()
                            archiveMutation.mutate({
                              id: conversation.id,
                              archived: showingArchived,
                            })
                          }}
                        >
                          <HugeiconsIcon
                            icon={
                              showingArchived
                                ? ArchiveRestoreIcon
                                : Archive01Icon
                            }
                            className='size-4'
                            strokeWidth={2}
                            aria-hidden='true'
                          />
                          {t(showingArchived ? 'Restore' : 'Archive')}
                        </Button>
                      ) : null}
                    </div>
                  </div>
                  <p className='text-muted-foreground line-clamp-2 text-xs leading-5'>
                    {safePreview}
                  </p>
                </article>
              </Fragment>
            )
          })}
        </div>
      )}
    </div>
  )
}

export function AssistantHistoryConversation(props: {
  conversation: AssistantConversationHistoryItem
  onContinue?: (detail: AssistantConversationHistoryDetail) => void
}) {
  const { t } = useTranslation()
  const historyQuery = useQuery({
    queryKey: ['assistant-conversation', props.conversation.id],
    queryFn: () => getAssistantConversationHistoryDetail(props.conversation.id),
    staleTime: 30_000,
    retry: false,
  })
  const status = assistantHistoryErrorStatus(historyQuery.error)
  if (historyQuery.isLoading) {
    return (
      <div
        className='grid gap-3'
        aria-label={t('Loading conversation history...')}
      >
        <Skeleton className='h-8 w-48' />
        <Skeleton className='h-20 w-full' />
        <Skeleton className='h-20 w-full' />
      </div>
    )
  }
  if (historyQuery.isError || !historyQuery.data) {
    const description =
      status === 403
        ? t('Conversation history is not available to this account.')
        : t('This conversation no longer exists or is unavailable.')
    return (
      <Alert variant='destructive'>
        <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} aria-hidden='true' />
        <AlertTitle>{t('Conversation history')}</AlertTitle>
        <AlertDescription>{description}</AlertDescription>
      </Alert>
    )
  }
  const conversation = historyQuery.data.conversation
  return (
    <div className='grid gap-3'>
      <div className='flex items-start justify-between gap-3'>
        <div className='min-w-0'>
          <p className='truncate text-sm font-medium'>{conversation.title}</p>
          {conversation.owner !== 'self' ? (
            <p className='text-muted-foreground mt-1 text-xs leading-5'>
              {t(
                'This history is available because the account has a lower access level. Private cards remain visible only to their owner.'
              )}
            </p>
          ) : null}
        </div>
        {props.onContinue &&
        conversation.owner === 'self' &&
        conversation.archived_at === 0 &&
        !conversation.restricted_at ? (
          <Button
            type='button'
            variant='ghost'
            size='sm'
            className='shrink-0'
            onClick={() => props.onContinue?.(historyQuery.data)}
          >
            {t('Continue')}
          </Button>
        ) : null}
      </div>
      {historyQuery.data.messages.map((message) => (
        <HistoryMessage key={message.id} message={message} />
      ))}
    </div>
  )
}
