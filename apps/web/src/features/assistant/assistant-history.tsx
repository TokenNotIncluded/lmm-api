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
import { Alert02Icon, ShieldKeyIcon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery } from '@tanstack/react-query'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { Response } from '@/components/ai-elements/response'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { toIntlLocale } from '@/i18n/languages'

import {
  getAssistantConversationHistory,
  getAssistantConversationHistoryDetail,
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
    <div className='grid gap-1 rounded-md border px-3 py-2'>
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
}) {
  const { t, i18n } = useTranslation()
  const historyQuery = useQuery({
    queryKey: ['assistant-conversations'],
    queryFn: getAssistantConversationHistory,
    enabled: props.active,
    staleTime: 30_000,
    retry: false,
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

  if (historyQuery.isLoading) {
    return (
      <div
        className='grid gap-3'
        aria-label={t('Loading conversation history...')}
      >
        <Skeleton className='h-16 w-full' />
        <Skeleton className='h-16 w-full' />
      </div>
    )
  }

  if (historyQuery.isError) {
    const description =
      status === 403
        ? t('Conversation history is not available to this account.')
        : status === 404
          ? t('This conversation no longer exists or is unavailable.')
          : t('Unable to load conversation history. Try again.')
    return (
      <Alert variant='destructive'>
        <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} aria-hidden='true' />
        <AlertTitle>{t('Conversation history')}</AlertTitle>
        <AlertDescription>{description}</AlertDescription>
      </Alert>
    )
  }

  if (conversations.length === 0) {
    return (
      <p className='text-muted-foreground py-8 text-center text-sm leading-6'>
        {t('No visible conversation history yet.')}
      </p>
    )
  }

  return (
    <div className='grid gap-3'>
      {conversations.map((conversation) => {
        const safePreview = redactAssistantMessageForDisplay(
          conversation.last_message_preview,
          t(
            'Sensitive content is hidden and can only be accessed from a private card.'
          )
        ).content
        return (
          <article
            key={conversation.id}
            className='grid gap-2 rounded-lg border p-3'
          >
            <div className='flex items-start justify-between gap-3'>
              <div className='min-w-0'>
                <p className='text-sm font-medium'>
                  {conversation.owner === 'self'
                    ? t('Your conversation')
                    : t('Lower-access user conversation')}
                </p>
                <p className='text-muted-foreground mt-0.5 text-xs'>
                  {dateFormatter.format(
                    new Date(conversation.updated_at * 1000)
                  )}
                </p>
              </div>
              <Button
                type='button'
                variant='outline'
                size='sm'
                onClick={() => props.onOpenConversation(conversation)}
              >
                {t('View')}
              </Button>
            </div>
            <p className='text-muted-foreground line-clamp-2 text-xs leading-5'>
              {safePreview}
            </p>
          </article>
        )
      })}
    </div>
  )
}

export function AssistantHistoryConversation(props: {
  conversation: AssistantConversationHistoryItem
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
      <div>
        <p className='text-sm font-medium'>
          {conversation.owner === 'self'
            ? t('Your conversation')
            : t('Lower-access user conversation')}
        </p>
        {conversation.owner !== 'self' ? (
          <p className='text-muted-foreground mt-1 text-xs leading-5'>
            {t(
              'This history is available because the account has a lower access level. Private cards remain visible only to their owner.'
            )}
          </p>
        ) : null}
      </div>
      {historyQuery.data.messages.map((message) => (
        <HistoryMessage key={message.id} message={message} />
      ))}
    </div>
  )
}
