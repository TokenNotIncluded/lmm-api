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
import { MessageSquareText } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { AssistantConversationHistoryItem } from '@/features/assistant/api'
import {
  AssistantHistory,
  AssistantHistoryConversation,
} from '@/features/assistant/assistant-history'
import { useAuthStore } from '@/stores/auth-store'

import { canViewUserAssistantHistory } from '../lib/assistant-history-access'
import type { User } from '../types'

export function UserAssistantHistoryDialog(props: { user: User }) {
  const { t } = useTranslation()
  const viewer = useAuthStore((state) => state.auth.user)
  const [open, setOpen] = useState(false)
  const [conversation, setConversation] =
    useState<AssistantConversationHistoryItem | null>(null)
  const count = props.user.assistant_conversation_count
  const canView = canViewUserAssistantHistory(viewer, props.user)

  if (!canView || count === undefined) {
    return <span className='text-muted-foreground'>—</span>
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen)
        if (!nextOpen) setConversation(null)
      }}
    >
      <Button
        type='button'
        variant='outline'
        size='sm'
        className='min-h-11 sm:min-h-7'
        onClick={() => setOpen(true)}
        aria-label={`${t('Support conversations')}: ${count}`}
      >
        <MessageSquareText aria-hidden='true' />
        {count.toLocaleString()}
      </Button>
      <DialogContent className='flex max-h-[min(52rem,calc(100svh-2rem))] min-h-0 w-[calc(100%-1rem)] flex-col sm:max-w-4xl'>
        <DialogHeader>
          <DialogTitle>{t('Support conversations')}</DialogTitle>
          <DialogDescription>
            <span className='block truncate'>
              {props.user.display_name || props.user.username} · @
              {props.user.username}
              {props.user.email ? ` · ${props.user.email}` : ''}
            </span>
            <span className='mt-1 block text-xs'>
              {t('User ID')}: {props.user.id} · {count.toLocaleString()}
            </span>
          </DialogDescription>
        </DialogHeader>
        <div className='min-h-0 overflow-y-auto pr-1'>
          {conversation ? (
            <div className='grid gap-4'>
              <div>
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  onClick={() => setConversation(null)}
                >
                  {t('Back')}
                </Button>
              </div>
              <AssistantHistoryConversation conversation={conversation} />
            </div>
          ) : (
            <AssistantHistory
              active={open}
              ownerUser={{ id: props.user.id, username: props.user.username }}
              presentation='rows'
              onOpenConversation={setConversation}
            />
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
