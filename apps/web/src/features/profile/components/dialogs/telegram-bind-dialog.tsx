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
import { Send } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { z } from 'zod'

import { Dialog } from '@/components/dialog'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { TELEGRAM_BIND_RESULT_MESSAGE } from '@/features/auth/constants'
import { AuthCookieBlockedError } from '@/lib/auth-cookie'
import { getServerErrorMessageKey } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import {
  getUserProfile,
  startTelegramBind,
  type TelegramBindFlow,
} from '../../api'

// ============================================================================
// Telegram Bind Dialog Component
// ============================================================================

interface TelegramBindDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  botName: string
  onSuccess: () => void | Promise<void>
}

type BindState =
  | { kind: 'idle' }
  | { kind: 'starting' }
  | { kind: 'ready'; flow: TelegramBindFlow }
  | { kind: 'refreshing' }
  | { kind: 'error'; messageKey: string }
  | { kind: 'blocked'; message: string }

type ActiveBindFlow = {
  flow: TelegramBindFlow
  generation: number
  ownerKey: string
  userId: number
}

const bindResultSchema = z.object({
  type: z.literal(TELEGRAM_BIND_RESULT_MESSAGE),
  flow_token: z.string().regex(/^lmmt_[0-9a-fA-F]{64}$/),
  success: z.boolean(),
  code: z.string().max(128).optional(),
})

const boundProfileSchema = z.object({
  success: z.literal(true),
  data: z.object({
    id: z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
    telegram_id: z
      .string()
      .regex(/^[1-9][0-9]{0,18}$/)
      .pipe(
        z.string().refine((value) => BigInt(value) <= 9223372036854775807n)
      ),
  }),
})

function currentOwnerKey() {
  const { user, session } = useAuthStore.getState().auth
  return JSON.stringify([user?.id ?? null, session?.sid ?? null])
}

function bindErrorState(
  value: unknown,
  fallback: string
): Extract<BindState, { kind: 'error' | 'blocked' }> {
  if (value instanceof AuthCookieBlockedError) {
    return { kind: 'blocked', message: value.message }
  }
  const messageKey = getServerErrorMessageKey(value)
  if (
    messageKey &&
    messageKey === getServerErrorMessageKey({ code: 'AUTH_USER_DISABLED' })
  ) {
    return { kind: 'error', messageKey: 'This user account is disabled.' }
  }
  return { kind: 'error', messageKey: messageKey || fallback }
}

export function TelegramBindDialog({
  open,
  onOpenChange,
  botName,
  onSuccess,
}: TelegramBindDialogProps) {
  const { t } = useTranslation()
  const { user, session } = useAuthStore((store) => store.auth)
  const ownerKey = JSON.stringify([user?.id ?? null, session?.sid ?? null])
  const widgetRef = useRef<HTMLDivElement>(null)
  const generation = useRef(0)
  const activeFlow = useRef<ActiveBindFlow | null>(null)
  const isOpen = useRef(open)
  isOpen.current = open
  const [state, setState] = useState<BindState>({ kind: 'idle' })

  const isCurrent = useCallback(
    (attempt: number, expectedOwner: string) =>
      isOpen.current &&
      generation.current === attempt &&
      currentOwnerKey() === expectedOwner,
    []
  )

  const invalidateFlow = useCallback(() => {
    generation.current++
    activeFlow.current = null
  }, [])

  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen) {
        invalidateFlow()
        isOpen.current = false
        setState({ kind: 'idle' })
      }
      onOpenChange(nextOpen)
    },
    [invalidateFlow, onOpenChange]
  )

  const createBindFlow = useCallback(async () => {
    const attempt = ++generation.current
    const expectedOwner = currentOwnerKey()
    const { user: initiatingUser, session: initiatingSession } =
      useAuthStore.getState().auth
    activeFlow.current = null
    setState({ kind: 'starting' })
    if (!initiatingUser || !initiatingSession) {
      setState({
        kind: 'error',
        messageKey: 'Failed to start Telegram binding',
      })
      return
    }
    try {
      const response = await startTelegramBind()
      if (!isCurrent(attempt, expectedOwner)) return
      if (!response.success) {
        setState(bindErrorState(response, 'Failed to start Telegram binding'))
        return
      }
      activeFlow.current = {
        flow: response.data,
        generation: attempt,
        ownerKey: expectedOwner,
        userId: initiatingUser.id,
      }
      setState({ kind: 'ready', flow: response.data })
    } catch (bindError: unknown) {
      if (!isCurrent(attempt, expectedOwner)) return
      setState(bindErrorState(bindError, 'Failed to start Telegram binding'))
    }
  }, [isCurrent])

  useEffect(() => {
    if (!open) {
      setState({ kind: 'idle' })
      return
    }
    void createBindFlow()
    return invalidateFlow
  }, [createBindFlow, invalidateFlow, open, ownerKey])

  useEffect(() => {
    if (!open) return

    const handleBindResult = (event: MessageEvent<unknown>) => {
      if (event.origin !== window.location.origin) return
      const pending = activeFlow.current
      const parsed = bindResultSchema.safeParse(event.data)
      if (
        !pending ||
        !parsed.success ||
        !isCurrent(pending.generation, pending.ownerKey) ||
        parsed.data.flow_token !== pending.flow.flow_token
      ) {
        return
      }
      activeFlow.current = null
      if (pending.flow.expires_at <= Date.now() / 1000) {
        setState({
          kind: 'error',
          messageKey:
            'This Telegram binding request has expired or has already been used.',
        })
        return
      }
      if (!parsed.data.success) {
        setState(
          bindErrorState(
            { code: parsed.data.code },
            'Telegram binding failed. Please try again.'
          )
        )
        return
      }
      setState({ kind: 'refreshing' })
      void (async () => {
        try {
          const response = await getUserProfile({
            timeout: 10_000,
            skipBusinessError: true,
            skipErrorHandler: true,
          })
          if (!isCurrent(pending.generation, pending.ownerKey)) return
          const actual = boundProfileSchema.safeParse(response)
          if (!actual.success || actual.data.data.id !== pending.userId) {
            setState({ kind: 'error', messageKey: 'Failed to load profile' })
            return
          }
          await onSuccess()
          if (!isCurrent(pending.generation, pending.ownerKey)) return
          toast.success(t('Binding successful!'))
          handleOpenChange(false)
        } catch (refreshError: unknown) {
          if (isCurrent(pending.generation, pending.ownerKey)) {
            setState(bindErrorState(refreshError, 'Failed to load profile'))
          }
        }
      })()
    }

    window.addEventListener('message', handleBindResult)
    return () => window.removeEventListener('message', handleBindResult)
  }, [handleOpenChange, isCurrent, onSuccess, open, t])

  useEffect(() => {
    if (!open || state.kind !== 'ready') return
    const pending = activeFlow.current
    if (!pending) return
    const timer = window.setTimeout(
      () => {
        if (!isCurrent(pending.generation, pending.ownerKey)) return
        activeFlow.current = null
        setState({
          kind: 'error',
          messageKey:
            'This Telegram binding request has expired or has already been used.',
        })
      },
      Math.max(0, pending.flow.expires_at * 1000 - Date.now())
    )
    return () => window.clearTimeout(timer)
  }, [isCurrent, open, state])

  useEffect(() => {
    const container = widgetRef.current
    if (!container || !open || state.kind !== 'ready') return

    container.replaceChildren()
    const script = document.createElement('script')
    script.async = true
    script.src = 'https://telegram.org/js/telegram-widget.js?22'
    script.setAttribute('data-telegram-login', botName.replace(/^@/, ''))
    script.setAttribute('data-size', 'large')
    script.setAttribute('data-auth-url', state.flow.callback_url)
    script.setAttribute('data-request-access', 'write')
    container.appendChild(script)

    return () => container.replaceChildren()
  }, [botName, open, state])

  return (
    <Dialog
      open={open}
      onOpenChange={handleOpenChange}
      title={t('Bind Telegram Account')}
      description={t('Click the button below to bind your Telegram account')}
      contentClassName='sm:max-w-md'
      contentHeight='auto'
      bodyClassName='space-y-4'
    >
      <div className='space-y-4 py-4'>
        <Alert>
          <Send className='h-4 w-4' />
          <AlertDescription>
            {t(
              'You will be redirected to Telegram to complete the binding process.'
            )}
          </AlertDescription>
        </Alert>

        <div className='flex flex-col items-center justify-center gap-4 rounded-lg border p-6'>
          <div className='console-status-info-surface flex h-12 w-12 items-center justify-center rounded-none'>
            <Send className='h-6 w-6' />
          </div>

          <div className='text-center'>
            <p className='text-muted-foreground text-sm'>
              {t('Bot:')}{' '}
              <span className='font-mono font-semibold'>@{botName}</span>
            </p>
            <p className='text-muted-foreground mt-1 text-xs'>
              {t(
                "After clicking the button, you'll be asked to authorize the bot"
              )}
            </p>
          </div>

          {state.kind === 'starting' && <Skeleton className='h-10 w-52' />}
          {state.kind === 'refreshing' && (
            <p role='status' className='text-muted-foreground text-sm'>
              {t('Refreshing...')}
            </p>
          )}
          {(state.kind === 'error' || state.kind === 'blocked') && (
            <div className='flex flex-col items-center gap-3 text-center'>
              <p role='alert' className='text-destructive text-sm'>
                {state.kind === 'blocked' ? state.message : t(state.messageKey)}
              </p>
              <Button type='button' variant='outline' onClick={createBindFlow}>
                {t('Retry')}
              </Button>
            </div>
          )}
          <div ref={widgetRef} className='flex min-h-10 justify-center' />
        </div>

        <p className='text-muted-foreground text-center text-xs'>
          {t('The binding will complete automatically after authorization')}
        </p>
      </div>
    </Dialog>
  )
}
