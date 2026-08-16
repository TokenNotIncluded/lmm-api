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
*/
import { ArrowLeft01Icon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

import type { AssistantConversationHistoryItem } from '../assistant/api'
import {
  AssistantHistory,
  AssistantHistoryConversation,
} from '../assistant/assistant-history'

/**
 * A calm, full-page home for assistant conversations. The launcher remains
 * useful for a quick exchange; this view is for users who have accumulated a
 * real history and need to browse it without a cramped side rail.
 */
export function ChatManagement() {
  const { t } = useTranslation()
  const [selected, setSelected] =
    useState<AssistantConversationHistoryItem | null>(null)

  return (
    <main className='h-full min-h-0 overflow-y-auto'>
      <div className='mx-auto grid w-full max-w-6xl gap-10 px-5 py-10 sm:px-8 lg:px-12'>
        <header className='grid gap-2'>
          <p className='text-muted-foreground text-xs tracking-[0.18em] uppercase'>
            {t('Chat')}
          </p>
          <h1 className='text-2xl font-medium tracking-tight sm:text-3xl'>
            {t('Chat management')}
          </h1>
          <p className='text-muted-foreground max-w-2xl text-sm leading-6'>
            {t(
              'Browse, continue, and archive your assistant conversations from one spacious place.'
            )}
          </p>
        </header>

        <div className='grid min-w-0 gap-10 lg:grid-cols-[minmax(0,1fr)_minmax(18rem,24rem)] lg:gap-12'>
          <section
            aria-labelledby='chat-history-heading'
            className={cn('min-w-0', selected && 'hidden lg:block')}
          >
            <div className='mb-4 flex items-center justify-between gap-3'>
              <h2
                id='chat-history-heading'
                className='text-base font-medium tracking-tight'
              >
                {t('Conversation history')}
              </h2>
              {selected ? (
                <Button
                  type='button'
                  variant='ghost'
                  size='sm'
                  onClick={() => setSelected(null)}
                >
                  <HugeiconsIcon
                    icon={ArrowLeft01Icon}
                    className='size-4'
                    strokeWidth={2}
                    aria-hidden='true'
                  />
                  {t('Back to list')}
                </Button>
              ) : null}
            </div>
            <AssistantHistory
              active
              presentation='rows'
              onOpenConversation={setSelected}
            />
          </section>

          <aside
            className={cn(
              'min-w-0 border-t pt-8 lg:border-t-0 lg:border-l lg:pt-0 lg:pl-10',
              !selected && 'hidden lg:block'
            )}
          >
            {selected ? (
              <div className='grid gap-4'>
                <Button
                  type='button'
                  variant='ghost'
                  size='sm'
                  className='w-fit px-0 lg:hidden'
                  data-testid='chat-management-back'
                  onClick={() => setSelected(null)}
                >
                  <HugeiconsIcon
                    icon={ArrowLeft01Icon}
                    className='size-4'
                    strokeWidth={2}
                    aria-hidden='true'
                  />
                  {t('Back to list')}
                </Button>
                <AssistantHistoryConversation conversation={selected} />
              </div>
            ) : (
              <div className='grid gap-2 py-1'>
                <p className='text-base font-medium'>
                  {t('Open a conversation')}
                </p>
                <p className='text-muted-foreground text-sm leading-6'>
                  {t(
                    'Select a conversation to read the full transcript. Private credentials remain protected.'
                  )}
                </p>
              </div>
            )}
          </aside>
        </div>
      </div>
    </main>
  )
}
