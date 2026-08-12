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
import { useIsMobile } from '@/hooks/use-mobile'
import { useStatus } from '@/hooks/use-status'
import { isConsoleActivated } from '@/lib/console-activation'
import { useAuthStore } from '@/stores/auth-store'

import {
  consumeQueuedAssistantPreset,
  consumeQueuedAssistantMessage,
  subscribeToAssistantOpen,
  type AssistantPresetId,
} from './assistant-events'

const loadAssistantPanel = () => import('./assistant-panel')
const AssistantPanel = lazy(() =>
  loadAssistantPanel().then((module) => ({
    default: module.AssistantPanel,
  }))
)

export function AssistantLauncher() {
  const { t } = useTranslation()
  const { status } = useStatus()
  const user = useAuthStore((state) => state.auth.user)
  const isMobile = useIsMobile()
  const [mobileOpen, setMobileOpen] = useState(false)
  const [desktopCollapsed, setDesktopCollapsed] = useState(false)
  const [desktopFullscreen, setDesktopFullscreen] = useState(false)
  const [initialPreset, setInitialPreset] = useState<AssistantPresetId>()
  const [initialMessage, setInitialMessage] = useState<string>()
  const [initialMessageRevision, setInitialMessageRevision] = useState(0)

  const showAssistant = useCallback(
    (preset?: AssistantPresetId, message?: string) => {
      const nextMessage = message ?? consumeQueuedAssistantMessage()
      setInitialPreset(preset)
      setInitialMessage(nextMessage)
      if (nextMessage?.trim()) {
        setInitialMessageRevision((revision) => revision + 1)
      }
      setDesktopCollapsed(false)
      setDesktopFullscreen(false)
      setMobileOpen(true)
    },
    []
  )

  const handleConversationReset = useCallback(() => {
    setInitialPreset(undefined)
    setInitialMessage(undefined)
    setInitialMessageRevision((revision) => revision + 1)
  }, [])

  useEffect(() => {
    const queuedPreset = consumeQueuedAssistantPreset()
    const queuedMessage = consumeQueuedAssistantMessage()
    if (queuedPreset || queuedMessage) {
      showAssistant(queuedPreset, queuedMessage)
    }
    return subscribeToAssistantOpen(showAssistant)
  }, [showAssistant])

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
      showAssistant()
    }

    window.addEventListener('keydown', handleShortcut)
    return () => window.removeEventListener('keydown', handleShortcut)
  }, [showAssistant])

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
      <div
        className='border-border bg-muted/20 flex min-h-14 w-full shrink-0 items-center border-t px-3 py-1.5 pb-[max(0.375rem,env(safe-area-inset-bottom))] md:hidden'
        data-testid='assistant-mobile-launcher'
      >
        <Button
          type='button'
          variant='secondary'
          className='h-11 w-full justify-start gap-2 px-3 shadow-sm'
          aria-label={accessibleLabel}
          title={accessibleLabel}
          aria-haspopup='dialog'
          aria-expanded={mobileOpen}
          aria-controls='ai-assistant-panel'
          data-testid='assistant-launcher'
          onClick={() => showAssistant()}
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

      <Suspense
        fallback={
          <aside
            className='bg-background hidden min-h-0 w-[min(28vw,30rem)] max-w-full min-w-0 shrink-0 border-l md:flex'
            aria-hidden='true'
          />
        }
      >
        <AssistantPanel
          mode={isMobile ? 'mobile' : 'rail'}
          open={isMobile ? mobileOpen : true}
          collapsed={!isMobile && desktopCollapsed}
          fullscreen={!isMobile && desktopFullscreen}
          initialPreset={initialPreset}
          initialMessage={initialMessage}
          initialMessageRevision={initialMessageRevision}
          onOpenChange={setMobileOpen}
          onConversationReset={handleConversationReset}
          onToggleCollapsed={() => setDesktopCollapsed((value) => !value)}
          onToggleFullscreen={() => setDesktopFullscreen((value) => !value)}
        />
      </Suspense>
    </div>
  )
}
