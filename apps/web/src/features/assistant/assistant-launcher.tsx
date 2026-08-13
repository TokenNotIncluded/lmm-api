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
import { AiChat02Icon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { lazy, Suspense, useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useStatus } from '@/hooks/use-status'
import { isConsoleActivated } from '@/lib/console-activation'
import { useAuthStore } from '@/stores/auth-store'

import {
  consumeQueuedAssistantRequest,
  peekQueuedAssistantRequest,
  subscribeToAssistantOpen,
  type AssistantPresetId,
} from './assistant-events'
import { useAssistantOverlay } from './assistant-responsive'

const loadAssistantPanel = () => import('./assistant-panel')
const AssistantPanel = lazy(() =>
  loadAssistantPanel().then((module) => ({
    default: module.AssistantPanel,
  }))
)

export function AssistantLauncher(props: { page?: boolean }) {
  const { t } = useTranslation()
  const { status } = useStatus()
  const user = useAuthStore((state) => state.auth.user)
  const assistantOverlay = useAssistantOverlay()
  const [mobileOpen, setMobileOpen] = useState(false)
  const [desktopCollapsed, setDesktopCollapsed] = useState(false)
  const [desktopFullscreen, setDesktopFullscreen] = useState(false)
  const [initialPreset, setInitialPreset] = useState<
    AssistantPresetId | undefined
  >(() =>
    props.page
      ? isConsoleActivated(user)
        ? 'client-setup'
        : 'onboarding'
      : undefined
  )
  const [initialMessage, setInitialMessage] = useState<string>()
  const [initialMessageRevision, setInitialMessageRevision] = useState(0)
  const [autoSendRequestId, setAutoSendRequestId] = useState<string>()

  const showAssistant = useCallback(
    (request: {
      id: string
      preset?: AssistantPresetId
      message?: string
      autoSend: boolean
    }) => {
      let preset = request.preset
      if (!preset && request.autoSend) {
        preset = isConsoleActivated(user) ? 'service' : 'onboarding'
      }
      setInitialPreset(preset)
      setInitialMessage(request.message)
      if (request.message?.trim()) {
        setInitialMessageRevision((revision) => revision + 1)
        setAutoSendRequestId(request.autoSend ? request.id : undefined)
      }
      if (!request.autoSend) consumeQueuedAssistantRequest(request.id)
      setDesktopCollapsed(false)
      setDesktopFullscreen(false)
      setMobileOpen(true)
    },
    [user]
  )

  const showManualAssistant = useCallback(() => {
    showAssistant({ id: 'manual', autoSend: false })
  }, [showAssistant])

  const handleConversationReset = useCallback(() => {
    setInitialPreset(undefined)
    setInitialMessage(undefined)
    setInitialMessageRevision((revision) => revision + 1)
    setAutoSendRequestId(undefined)
  }, [])

  useEffect(() => {
    const queued = peekQueuedAssistantRequest()
    if (queued) showAssistant(queued)
    return subscribeToAssistantOpen(showAssistant)
  }, [showAssistant])

  const handleAutoSendConsumed = useCallback((requestId: string) => {
    consumeQueuedAssistantRequest(requestId)
    setAutoSendRequestId((current) =>
      current === requestId ? undefined : current
    )
  }, [])

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.altKey ||
        !event.shiftKey ||
        !(event.metaKey || event.ctrlKey) ||
        event.key.toLowerCase() !== 'a'
      ) {
        return
      }

      event.preventDefault()
      showManualAssistant()
    }

    window.addEventListener('keydown', handleShortcut)
    return () => window.removeEventListener('keydown', handleShortcut)
  }, [showManualAssistant])

  if (status?.assistant?.enabled === false) return null

  const needsL1Unlock = user !== null && !isConsoleActivated(user)
  const visibleLabel = needsL1Unlock
    ? t('Unlock L1 with AI')
    : t('Service guide')
  const accessibleLabel = needsL1Unlock
    ? t('Unlock L1 with AI')
    : t('Open AI assistant')

  return (
    <div className='contents'>
      {props.page ? null : (
        <div
          className='border-border bg-muted/20 pointer-events-none fixed inset-x-0 bottom-0 z-40 flex min-h-14 items-center justify-center border-t px-3 py-1.5 pb-[max(0.375rem,env(safe-area-inset-bottom))] md:inset-x-auto md:right-4 md:bottom-4 md:min-h-0 md:w-auto md:justify-end md:border md:border-none md:bg-transparent md:px-0 md:py-0 md:pb-0 xl:hidden'
          data-testid='assistant-mobile-launcher'
        >
          <Button
            type='button'
            variant='secondary'
            className='pointer-events-auto h-11 w-full max-w-md justify-start gap-2 px-3 shadow-sm md:w-auto md:min-w-44'
            aria-label={accessibleLabel}
            title={accessibleLabel}
            aria-haspopup='dialog'
            aria-expanded={mobileOpen}
            aria-controls='ai-assistant-panel'
            data-testid='assistant-launcher'
            onClick={showManualAssistant}
          >
            <HugeiconsIcon
              icon={AiChat02Icon}
              strokeWidth={2}
              data-icon='inline-start'
              aria-hidden='true'
            />
            <span className='truncate text-sm font-medium'>{visibleLabel}</span>
          </Button>
        </div>
      )}

      <Suspense
        fallback={
          <aside
            className={
              props.page
                ? 'bg-background flex min-h-0 w-full flex-1'
                : 'bg-background hidden min-h-0 w-[min(28vw,30rem)] max-w-full min-w-0 shrink-0 border-l xl:flex'
            }
            aria-hidden='true'
          />
        }
      >
        <AssistantPanel
          mode={props.page ? 'page' : assistantOverlay ? 'mobile' : 'rail'}
          open={props.page || (assistantOverlay ? mobileOpen : true)}
          collapsed={!assistantOverlay && desktopCollapsed}
          fullscreen={!assistantOverlay && desktopFullscreen}
          initialPreset={initialPreset}
          initialMessage={initialMessage}
          initialMessageRevision={initialMessageRevision}
          autoSendRequestId={autoSendRequestId}
          onAutoSendConsumed={handleAutoSendConsumed}
          onOpenChange={setMobileOpen}
          onConversationReset={handleConversationReset}
          onToggleCollapsed={() => setDesktopCollapsed((value) => !value)}
          onToggleFullscreen={() => setDesktopFullscreen((value) => !value)}
        />
      </Suspense>
    </div>
  )
}
