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
  ArrowRight01Icon,
  CleanIcon,
  Maximize01Icon,
  Minimize01Icon,
  ReloadIcon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { nanoid } from 'nanoid'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  Conversation,
  ConversationContent,
  ConversationScrollButton,
} from '@/components/ai-elements/conversation'
import { Loader } from '@/components/ai-elements/loader'
import { Message, MessageContent } from '@/components/ai-elements/message'
import {
  PromptInput,
  PromptInputProvider,
  PromptInputBody,
  PromptInputFooter,
  PromptInputSubmit,
  PromptInputTextarea,
  usePromptInputController,
} from '@/components/ai-elements/prompt-input'
import { Response } from '@/components/ai-elements/response'
import {
  sideDrawerContentClassName,
  sideDrawerHeaderClassName,
} from '@/components/drawer-layout'
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
import { Separator } from '@/components/ui/separator'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { useAuthStore } from '@/stores/auth-store'

import {
  getAssistantAvailableModels,
  getAssistantStatus,
  sendAssistantMessage,
  type AssistantConversationHistoryItem,
  type AssistantChatMessage,
  type AssistantAccountDisableAction,
  type AssistantAdminChangeAction,
  type AssistantL1RecommendationAction,
} from './api'
import {
  getAssistantAccountAccessState,
  type AssistantAccountAccessState,
} from './assistant-access'
import { AssistantAccountActionTool } from './assistant-account-action-tool'
import { AssistantActivationTool } from './assistant-activation-tool'
import { AssistantAdminChangeTool } from './assistant-admin-change-tool'
import { AssistantCostTool } from './assistant-cost-tool'
import {
  subscribeToAssistantOpen,
  type AssistantPresetId,
} from './assistant-events'
import { AssistantHandoffTool } from './assistant-handoff-tool'
import {
  AssistantHistory,
  AssistantHistoryConversation,
} from './assistant-history'
import { getAssistantPresetForIntent } from './assistant-intent'
import { AssistantKeyTool } from './assistant-key-tool'
import { redactAssistantMessageForDisplay } from './assistant-message-safety'
import { AssistantModelsTool } from './assistant-models-tool'
import { AssistantOnboardingTodo } from './assistant-onboarding-todo'
import { AssistantPlanTool } from './assistant-plan-tool'
import { getAssistantPromptValidation } from './assistant-prompt-validation'
import { AssistantSetupTool } from './assistant-setup-tool'
import { AssistantUsageTool } from './assistant-usage-tool'

type AssistantActionPath =
  | '/getting-started'
  | '/pricing'
  | '/wallet'
  | '/usage-logs'
  | '/keys'
  | '/open-source-bounties'
  | '/support'

type AssistantAction =
  | { kind: 'route'; label: string; to: AssistantActionPath }
  | { kind: 'email'; label: string; href: string }
  | {
      kind: 'tool'
      label: string
      tool:
        | 'activation'
        | 'key'
        | 'cost'
        | 'handoff'
        | 'models'
        | 'plan'
        | 'setup'
        | 'usage'
    }

type ConversationEntry = {
  id: string
  role: 'user' | 'assistant'
  content: string
  action?: AssistantAction
  adminChange?: AssistantAdminChangeAction
  error?: boolean
  retry?: {
    message: string
    history: AssistantChatMessage[]
  }
}

type AssistantPanelMode = 'mobile' | 'rail'

function getBaseUrl(): string {
  if (typeof window === 'undefined') return 'https://api.lmm.best/v1'
  return `${window.location.origin}/v1`
}

const ASSISTANT_PRIVACY_NOTICE_COLLAPSE_DELAY_MS = 5_000

function AssistantActionButton(props: {
  action: AssistantAction
  onToolOpen: (
    tool:
      | 'activation'
      | 'key'
      | 'cost'
      | 'handoff'
      | 'models'
      | 'plan'
      | 'setup'
      | 'usage'
  ) => void
}) {
  const { action } = props
  if (action.kind === 'tool') {
    return (
      <Button variant='outline' onClick={() => props.onToolOpen(action.tool)}>
        {action.label}
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          data-icon='inline-end'
          aria-hidden='true'
        />
      </Button>
    )
  }
  if (action.kind === 'email') {
    return (
      <Button variant='outline' render={<a href={action.href} />}>
        {action.label}
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          data-icon='inline-end'
          aria-hidden='true'
        />
      </Button>
    )
  }

  return (
    <Button variant='outline' render={<Link to={action.to} />}>
      {action.label}
      <HugeiconsIcon
        icon={ArrowRight01Icon}
        strokeWidth={2}
        data-icon='inline-end'
        aria-hidden='true'
      />
    </Button>
  )
}

type AssistantTool = Exclude<
  Extract<AssistantAction, { kind: 'tool' }>['tool'],
  never
>

function getAssistantToolForTarget(
  target: AssistantPresetId | undefined
): AssistantTool | null {
  switch (target) {
    case 'onboarding':
      return 'activation'
    case 'api-key':
      return 'key'
    case 'client-setup':
      return 'setup'
    case 'cost':
      return 'cost'
    case 'human':
      return 'handoff'
    case 'models':
      return 'models'
    case 'plan':
      return 'plan'
    case 'usage':
      return 'usage'
    default:
      return null
  }
}

function getAssistantActionForTarget(
  target: AssistantPresetId | undefined,
  translate: (key: string) => string
): AssistantAction | undefined {
  switch (target) {
    case 'onboarding':
      return {
        kind: 'tool',
        label: translate('Unlock L1 access'),
        tool: 'activation',
      }
    case 'plan':
      return {
        kind: 'tool',
        label: translate('Compare live plans'),
        tool: 'plan',
      }
    case 'api-key':
      return {
        kind: 'tool',
        label: translate('View connection details'),
        tool: 'key',
      }
    case 'client-setup':
      return {
        kind: 'tool',
        label: translate('Open client setup guide'),
        tool: 'setup',
      }
    case 'bounty':
      return {
        kind: 'route',
        label: translate('Explore open-source bounties'),
        to: '/open-source-bounties',
      }
    case 'cost':
      return {
        kind: 'tool',
        label: translate('Calculate with live pricing'),
        tool: 'cost',
      }
    case 'usage':
      return {
        kind: 'tool',
        label: translate('Open usage statistics'),
        tool: 'usage',
      }
    case 'models':
      return {
        kind: 'tool',
        label: translate('View all currently available models'),
        tool: 'models',
      }
    case 'invitation':
      return {
        kind: 'route',
        label: translate('Open wallet and invitations'),
        to: '/wallet',
      }
    case 'human':
      return {
        kind: 'tool',
        label: translate('Send to an administrator'),
        tool: 'handoff',
      }
    default:
      return undefined
  }
}

function AssistantAccountStatusNotice(props: {
  state: Extract<AssistantAccountAccessState, 'loading' | 'error'>
  onRetry: () => void
}) {
  const { t } = useTranslation()
  if (props.state === 'loading') {
    return (
      <Card size='sm' aria-label={t('Loading...')}>
        <CardHeader>
          <CardTitle className='sr-only'>{t('Loading...')}</CardTitle>
          <Skeleton className='h-4 w-44' />
        </CardHeader>
        <CardContent>
          <Skeleton className='h-9 w-full' />
        </CardContent>
      </Card>
    )
  }

  return (
    <Alert variant='destructive'>
      <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} aria-hidden='true' />
      <AlertTitle>{t('Unable to verify account access')}</AlertTitle>
      <AlertDescription>
        {t('Retry before using account-specific assistant tools.')}
      </AlertDescription>
      <AlertAction>
        <Button variant='outline' size='sm' onClick={props.onRetry}>
          {t('Retry')}
        </Button>
      </AlertAction>
    </Alert>
  )
}

function AssistantPromptInputSync(props: {
  initialMessage?: string
  initialMessageRevision?: number
}) {
  const {
    textInput: { setInput },
  } = usePromptInputController()
  const initialMessage = props.initialMessage?.trim() ?? ''
  const lastRequest = useRef<{
    message: string
    revision?: number
  } | null>(null)

  useEffect(() => {
    if (
      lastRequest.current?.message === initialMessage &&
      lastRequest.current?.revision === props.initialMessageRevision
    ) {
      return
    }

    lastRequest.current = {
      message: initialMessage,
      revision: props.initialMessageRevision,
    }
    if (initialMessage) setInput(initialMessage)
  }, [initialMessage, props.initialMessageRevision, setInput])

  return null
}

function AssistantPromptComposer(props: {
  footerStatus: string
  placeholder: string
  privacyNoticeId: string
  restricted: boolean
  sending: boolean
  onSubmit: (message: { text?: string }) => void | Promise<void>
}) {
  const { t } = useTranslation()
  const {
    textInput: { value },
  } = usePromptInputController()
  const validation = getAssistantPromptValidation(value, props.restricted)
  const hasText = value.trim().length > 0
  const showValidationError = hasText && validation.invalid
  const hintId = 'assistant-l0-input-hint'
  const describedBy =
    props.restricted || showValidationError
      ? `${props.privacyNoticeId} ${hintId}`
      : props.privacyNoticeId

  return (
    <>
      <PromptInput
        onSubmit={props.onSubmit}
        groupClassName='rounded-xl'
        aria-label={t('Ask AI assistant')}
      >
        <PromptInputBody>
          <PromptInputTextarea
            placeholder={props.placeholder}
            maxLength={4000}
            required={props.restricted}
            aria-describedby={describedBy}
            aria-invalid={hasText ? validation.invalid : undefined}
            disabled={props.sending}
            className='max-h-32 min-h-12'
          />
        </PromptInputBody>
        <PromptInputFooter>
          <span className='text-muted-foreground min-w-0 flex-1 truncate text-xs'>
            {props.footerStatus}
          </span>
          <PromptInputSubmit
            status={props.sending ? 'submitted' : 'ready'}
            disabled={props.sending || validation.invalid}
          />
        </PromptInputFooter>
      </PromptInput>
      {props.restricted || showValidationError ? (
        <p
          id={hintId}
          className={
            showValidationError
              ? 'text-destructive mt-2 px-1 text-xs leading-5'
              : 'text-muted-foreground mt-2 px-1 text-xs leading-5'
          }
          role={showValidationError ? 'alert' : 'status'}
          aria-live='polite'
        >
          {showValidationError
            ? t('Please enter a message other than a single punctuation mark.')
            : t(
                'Write a short explanation of what you want to build or why you need L1 access.'
              )}
        </p>
      ) : null}
    </>
  )
}

function AssistantPanelHeader(props: {
  mode: AssistantPanelMode
  description: string
  historyVisible: boolean
  onOpenHistory: () => void
  onCloseHistory: () => void
  onToggleCollapsed?: () => void
  fullscreen?: boolean
  onToggleFullscreen?: () => void
}) {
  const { t } = useTranslation()

  if (props.mode === 'mobile') {
    return (
      <SheetHeader
        className={sideDrawerHeaderClassName(
          'shrink-0 pr-12 pt-[max(0.75rem,env(safe-area-inset-top))]'
        )}
      >
        <SheetTitle>{t('Service guide')}</SheetTitle>
        <SheetDescription>{props.description}</SheetDescription>
        <Button
          type='button'
          variant='outline'
          size='sm'
          className='mt-2 self-start'
          onClick={
            props.historyVisible ? props.onCloseHistory : props.onOpenHistory
          }
        >
          {props.historyVisible
            ? t('Back to conversation')
            : t('Conversation history')}
        </Button>
      </SheetHeader>
    )
  }

  return (
    <header className='border-border/70 bg-background/95 supports-[backdrop-filter]:bg-background/80 flex min-w-0 shrink-0 items-start gap-2 border-b px-3 py-3 backdrop-blur sm:gap-3 sm:px-4'>
      <div className='min-w-0 flex-1'>
        <h2 className='truncate text-base leading-6 font-semibold'>
          {t('Service guide')}
        </h2>
        <p className='text-muted-foreground mt-0.5 text-xs leading-5'>
          {props.description}
        </p>
      </div>
      <div className='flex min-w-0 shrink-0 items-center gap-0.5'>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='max-w-32 shrink-0 truncate px-2 sm:max-w-40'
          onClick={
            props.historyVisible ? props.onCloseHistory : props.onOpenHistory
          }
        >
          {props.historyVisible ? t('Back') : t('Conversation history')}
        </Button>
        {!props.fullscreen ? (
          <Button
            type='button'
            variant='ghost'
            size='icon-sm'
            aria-label={t('Collapse')}
            title={t('Collapse')}
            data-testid='assistant-collapse'
            onClick={props.onToggleCollapsed}
          >
            <HugeiconsIcon
              icon={ArrowRight01Icon}
              strokeWidth={2}
              aria-hidden='true'
            />
          </Button>
        ) : null}
        <Button
          type='button'
          variant='ghost'
          size='icon-sm'
          aria-label={
            props.fullscreen ? t('Exit full screen') : t('Enter full screen')
          }
          title={
            props.fullscreen ? t('Exit full screen') : t('Enter full screen')
          }
          data-testid='assistant-fullscreen'
          onClick={props.onToggleFullscreen}
        >
          <HugeiconsIcon
            icon={props.fullscreen ? Minimize01Icon : Maximize01Icon}
            strokeWidth={2}
            aria-hidden='true'
          />
        </Button>
      </div>
    </header>
  )
}

export function AssistantPanel(props: {
  open: boolean
  mode?: AssistantPanelMode
  collapsed?: boolean
  fullscreen?: boolean
  initialPreset?: AssistantPresetId
  initialMessage?: string
  initialMessageRevision?: number
  onOpenChange: (open: boolean) => void
  onConversationReset?: () => void
  onToggleCollapsed?: () => void
  onToggleFullscreen?: () => void
}) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const mode = props.mode ?? 'mobile'
  const onConversationReset = props.onConversationReset
  const panelVisible =
    mode === 'rail' ? !props.collapsed || props.fullscreen === true : props.open
  const baseUrl = getBaseUrl()
  const [entries, setEntries] = useState<ConversationEntry[]>([])
  const [sending, setSending] = useState(false)
  const [recommendationDraft, setRecommendationDraft] =
    useState<AssistantL1RecommendationAction | null>(null)
  const [accountDisableDraft, setAccountDisableDraft] =
    useState<AssistantAccountDisableAction | null>(null)
  const [historyView, setHistoryView] = useState<
    'list' | AssistantConversationHistoryItem | null
  >(null)
  const [activeTool, setActiveTool] = useState<
    | 'activation'
    | 'key'
    | 'cost'
    | 'handoff'
    | 'models'
    | 'plan'
    | 'setup'
    | 'usage'
    | null
  >(null)
  const openedTargetRef = useRef<AssistantPresetId | undefined>(undefined)
  const [conversationResetRevision, setConversationResetRevision] = useState(0)
  const [privacyNoticeExpanded, setPrivacyNoticeExpanded] = useState(true)
  const privacyNoticeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null
  )
  const statusQuery = useQuery({
    queryKey: ['assistant-status'],
    queryFn: getAssistantStatus,
    enabled: panelVisible,
    staleTime: 30_000,
    retry: false,
  })
  const accountAccessState = getAssistantAccountAccessState(
    statusQuery.data,
    statusQuery.isError
  )
  const accountAccessConfirmed =
    accountAccessState === 'granted' || accountAccessState === 'restricted'
  const developerAccessGranted = accountAccessState === 'granted'
  const previousDeveloperAccessRef = useRef(developerAccessGranted)
  const authUser = useAuthStore((state) => state.auth.user)
  const clearPrivacyNoticeTimer = useCallback(() => {
    if (privacyNoticeTimerRef.current === null) return
    clearTimeout(privacyNoticeTimerRef.current)
    privacyNoticeTimerRef.current = null
  }, [])
  const schedulePrivacyNoticeCollapse = useCallback(() => {
    clearPrivacyNoticeTimer()
    setPrivacyNoticeExpanded(true)
    privacyNoticeTimerRef.current = setTimeout(() => {
      privacyNoticeTimerRef.current = null
      setPrivacyNoticeExpanded(false)
    }, ASSISTANT_PRIVACY_NOTICE_COLLAPSE_DELAY_MS)
  }, [clearPrivacyNoticeTimer])
  useEffect(() => {
    if (!panelVisible) {
      clearPrivacyNoticeTimer()
      return
    }

    schedulePrivacyNoticeCollapse()
    return clearPrivacyNoticeTimer
  }, [clearPrivacyNoticeTimer, panelVisible, schedulePrivacyNoticeCollapse])
  const togglePrivacyNotice = useCallback(() => {
    if (privacyNoticeExpanded) {
      clearPrivacyNoticeTimer()
      setPrivacyNoticeExpanded(false)
      return
    }
    schedulePrivacyNoticeCollapse()
  }, [
    clearPrivacyNoticeTimer,
    privacyNoticeExpanded,
    schedulePrivacyNoticeCollapse,
  ])
  const isAdministrator = statusQuery.data?.is_admin === true
  const accessLevel = statusQuery.data?.access_level
  const withAccessLevel = (message: string) =>
    accessLevel ? `${accessLevel} · ${message}` : message
  const superAdministratorFunded =
    statusQuery.data?.funding?.mode === 'super_administrator'
  const connectionModelsQuery = useQuery({
    queryKey: ['assistant-available-models'],
    queryFn: getAssistantAvailableModels,
    enabled:
      props.open &&
      developerAccessGranted &&
      (activeTool === 'key' || activeTool === 'setup'),
    staleTime: 60_000,
    retry: false,
  })
  const accountToolActive = activeTool !== null
  const historyVisible = historyView !== null

  useEffect(() => {
    const wasGranted = previousDeveloperAccessRef.current
    previousDeveloperAccessRef.current = developerAccessGranted
    if (
      wasGranted ||
      !developerAccessGranted ||
      activeTool !== 'activation' ||
      !panelVisible
    ) {
      return
    }

    setActiveTool(null)
    void navigate({ to: '/dashboard' })
  }, [activeTool, developerAccessGranted, navigate, panelVisible])

  let assistantFooterStatus = t('Loading...')
  let assistantDescription = t('Loading...')
  let assistantPromptPlaceholder = t('Ask AI assistant')
  if (isAdministrator) {
    assistantFooterStatus = withAccessLevel(t('Administrator mode'))
    assistantDescription = t(
      'Administrator mode can inspect safe server settings and prepare model pricing changes for your confirmation.'
    )
    assistantPromptPlaceholder = t(
      'Ask about server configuration, model pricing, or operations...'
    )
  } else if (developerAccessGranted) {
    assistantFooterStatus = withAccessLevel(
      superAdministratorFunded
        ? t('Funded by the super administrator')
        : t('Loading...')
    )
    assistantDescription = t(
      'Guidance for plans, setup, API keys, costs, and support.'
    )
    assistantPromptPlaceholder = t('Ask about plans, setup, keys, or costs...')
  } else if (accountAccessState === 'restricted') {
    assistantFooterStatus = withAccessLevel(
      superAdministratorFunded
        ? t('Funded by the super administrator')
        : t('Read-only')
    )
    assistantDescription = t(
      'L0 accounts can browse challenges and ask the AI assistant to request L1 access.'
    )
    assistantPromptPlaceholder = t(
      'Write a short explanation of what you want to build or why you need L1 access.'
    )
  } else if (accountAccessState === 'error') {
    assistantFooterStatus = withAccessLevel(
      t('Unable to verify account access')
    )
    assistantDescription = t('Unable to verify account access')
  }

  const clearToolState = useCallback(() => {
    setActiveTool(null)
    setRecommendationDraft(null)
    setAccountDisableDraft(null)
  }, [])

  const resetConversation = useCallback(() => {
    setEntries([])
    clearToolState()
    setHistoryView(null)
    openedTargetRef.current = undefined
    setConversationResetRevision((revision) => revision + 1)
    onConversationReset?.()
  }, [clearToolState, onConversationReset])

  const openAssistantTarget = useCallback(
    (target: AssistantPresetId) => {
      const restrictedTarget =
        target === 'onboarding' ||
        target === 'client-setup' ||
        target === 'bounty' ||
        target === 'cost'
      if (
        accountAccessState !== 'granted' &&
        !(accountAccessState === 'restricted' && restrictedTarget)
      ) {
        return false
      }

      const tool = getAssistantToolForTarget(target)
      const action = getAssistantActionForTarget(target, t)
      setActiveTool(tool)
      setEntries((current) => [
        ...current,
        {
          id: nanoid(),
          role: 'assistant',
          content: assistantDescription,
          action: tool ? undefined : action,
        },
      ])
      return true
    },
    [accountAccessState, assistantDescription, t]
  )

  useEffect(() => {
    const target = props.initialPreset
    if (!target || !accountAccessConfirmed) return
    if (openedTargetRef.current === target) return
    if (openAssistantTarget(target)) openedTargetRef.current = target
  }, [accountAccessConfirmed, openAssistantTarget, props.initialPreset])

  useEffect(
    () =>
      subscribeToAssistantOpen((target) => {
        if (!target) return
        if (openAssistantTarget(target)) openedTargetRef.current = target
      }),
    [openAssistantTarget]
  )

  const handleOpenChange = (open: boolean) => {
    props.onOpenChange(open)
  }

  const requestAssistantReply = async (
    message: string,
    history: AssistantChatMessage[]
  ) => {
    setSending(true)
    try {
      const reply = await sendAssistantMessage(message, history)
      const safeReply = redactAssistantMessageForDisplay(
        reply.content,
        t(
          'Sensitive content is hidden and can only be accessed from a private card.'
        )
      )
      const suggestedTarget = getAssistantPresetForIntent(reply.intent)
      const adminChange =
        reply.action?.type === 'admin_config_change' ||
        reply.action?.type === 'admin_pricing_change'
          ? reply.action
          : undefined
      let suggestedAction: AssistantAction | undefined
      if (developerAccessGranted || suggestedTarget === 'onboarding') {
        suggestedAction = getAssistantActionForTarget(suggestedTarget, t)
      }
      if (adminChange) {
        setRecommendationDraft(null)
        setAccountDisableDraft(null)
        setActiveTool(null)
        suggestedAction = undefined
      } else if (reply.action?.type === 'l1_recommendation') {
        setRecommendationDraft(reply.action)
        setAccountDisableDraft(null)
        setActiveTool('activation')
        suggestedAction = {
          kind: 'tool',
          label: t('Review AI recommendation'),
          tool: 'activation',
        }
      } else if (reply.action?.type === 'account_disable_request') {
        setAccountDisableDraft(reply.action)
        setRecommendationDraft(null)
        setActiveTool(null)
        suggestedAction = undefined
      }
      setEntries((current) => [
        ...current,
        {
          id: nanoid(),
          role: 'assistant',
          content: safeReply.content,
          action: suggestedAction,
          adminChange,
        },
      ])
      await queryClient.invalidateQueries({ queryKey: ['assistant-status'] })
    } catch {
      setEntries((current) => [
        ...current,
        {
          id: nanoid(),
          role: 'assistant',
          content: t(
            'The AI assistant could not answer right now. Try again or contact support.'
          ),
          error: true,
          retry: { message, history },
          action: developerAccessGranted
            ? {
                kind: 'route',
                label: t('Contact support'),
                to: '/support',
              }
            : undefined,
        },
      ])
    } finally {
      setSending(false)
    }
  }

  const submitMessage = async ({ text }: { text?: string }) => {
    const message = text?.trim()
    if (sending) return
    if (!message) {
      throw new Error(t('Please enter a message.'))
    }
    const validation = getAssistantPromptValidation(message)
    if (validation.invalid) {
      throw new Error(
        t('Please enter a message other than a single punctuation mark.')
      )
    }
    clearToolState()
    const safeMessage = redactAssistantMessageForDisplay(
      message,
      t(
        'Sensitive content is hidden and can only be accessed from a private card.'
      )
    )
    if (safeMessage.redacted) {
      setEntries((current) => [
        ...current,
        {
          id: nanoid(),
          role: 'assistant',
          content: t(
            'Sensitive message was not sent. Use the secure private card for credentials.'
          ),
          error: true,
        },
      ])
      return
    }
    const history: AssistantChatMessage[] = entries
      .filter((entry) => !entry.error)
      .map((entry) => ({ role: entry.role, content: entry.content }))
    setEntries((current) => [
      ...current,
      { id: nanoid(), role: 'user', content: safeMessage.content },
    ])
    await requestAssistantReply(safeMessage.content, history)
  }

  const retryMessage = async (entry: ConversationEntry) => {
    if (!entry.retry || sending) return
    setEntries((current) => current.filter((item) => item.id !== entry.id))
    await requestAssistantReply(entry.retry.message, entry.retry.history)
  }

  const panelContent = (
    <>
      <AssistantPanelHeader
        mode={mode}
        description={assistantDescription}
        historyVisible={historyVisible}
        onOpenHistory={() => setHistoryView('list')}
        onCloseHistory={() => setHistoryView(null)}
        onToggleCollapsed={props.onToggleCollapsed}
        fullscreen={props.fullscreen}
        onToggleFullscreen={props.onToggleFullscreen}
      />
      <Alert
        id='assistant-privacy-notice'
        className={
          privacyNoticeExpanded
            ? 'm-3 mb-0 max-w-[calc(100%-1.5rem)] min-w-0'
            : 'm-3 mb-0 max-w-[calc(100%-1.5rem)] min-w-0 py-1.5'
        }
        data-testid='assistant-privacy-notice'
        variant='default'
      >
        <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} aria-hidden='true' />
        <div className='min-w-0'>
          <AlertTitle>
            <button
              type='button'
              className='focus-visible:ring-ring/50 rounded-sm text-left font-medium outline-none focus-visible:ring-3'
              aria-controls='assistant-privacy-notice-description'
              aria-describedby='assistant-privacy-notice-description'
              aria-expanded={privacyNoticeExpanded}
              data-testid='assistant-privacy-notice-toggle'
              onClick={togglePrivacyNotice}
            >
              {t('Conversation privacy notice')}
            </button>
          </AlertTitle>
          <AlertDescription
            id='assistant-privacy-notice-description'
            className={privacyNoticeExpanded ? undefined : 'sr-only'}
          >
            <p>
              {t(
                'Your assistant conversations are not private. Authorized higher-access users may review them.'
              )}
            </p>
            <p>
              {t(
                'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials such as API keys are shown in a shielded private card and are kept out of the assistant context.'
              )}
            </p>
            <p>
              {t(
                'If you accidentally send supported sensitive data, the assistant safety filter may detect common email addresses, phone numbers, and API key formats and redact the message before this assistant request is sent. Pattern matching is not a guarantee.'
              )}
            </p>
          </AlertDescription>
        </div>
      </Alert>
      {historyVisible ? (
        <Conversation className='bg-muted/20 min-w-0'>
          <ConversationContent className='flex min-h-full max-w-full min-w-0 flex-col gap-5 px-4 py-5 sm:px-6'>
            {historyView === 'list' ? (
              <AssistantHistory
                active={panelVisible}
                onOpenConversation={(conversation) =>
                  setHistoryView(conversation)
                }
              />
            ) : (
              <AssistantHistoryConversation conversation={historyView} />
            )}
          </ConversationContent>
        </Conversation>
      ) : (
        <>
          {developerAccessGranted && authUser ? (
            <AssistantOnboardingTodo
              userId={authUser.id}
              enabled={developerAccessGranted}
              onOpenKey={() => {
                clearToolState()
                setActiveTool('key')
              }}
              onOpenSetup={() => {
                clearToolState()
                setActiveTool('setup')
              }}
            />
          ) : null}
          <Conversation className='bg-muted/20 min-w-0'>
            <ConversationContent className='flex min-h-full max-w-full min-w-0 flex-col gap-5 px-4 py-5 sm:px-6'>
              {entries.length === 0 ? (
                <div className='flex flex-1 flex-col gap-5'>
                  {accountAccessState === 'loading' ||
                  accountAccessState === 'error' ? (
                    <AssistantAccountStatusNotice
                      state={
                        accountAccessState === 'error' ? 'error' : 'loading'
                      }
                      onRetry={() => void statusQuery.refetch()}
                    />
                  ) : null}
                  {accountAccessState === 'restricted' ? (
                    <Card
                      size='sm'
                      className='border-primary/30 bg-primary/5'
                      data-testid='assistant-l0-welcome'
                    >
                      <CardHeader className='gap-2'>
                        <Badge variant='secondary' className='w-fit'>
                          {t('Read-only')}
                        </Badge>
                        <CardTitle className='text-lg'>
                          {t('Tell the AI assistant what you want to do')}
                        </CardTitle>
                        <CardDescription>
                          {t(
                            'L0 accounts can browse challenges and ask the AI assistant to request L1 access.'
                          )}
                        </CardDescription>
                      </CardHeader>
                      <CardContent className='grid gap-3'>
                        <p className='text-muted-foreground text-sm leading-6'>
                          {t(
                            'Write a short explanation of what you want to build or why you need L1 access.'
                          )}
                        </p>
                      </CardContent>
                    </Card>
                  ) : (
                    <div>
                      <p className='text-base font-medium'>
                        {t('How can I help?')}
                      </p>
                      <p className='text-muted-foreground mt-1 text-sm leading-6'>
                        {assistantDescription}
                      </p>
                    </div>
                  )}
                </div>
              ) : (
                <>
                  {entries.map((entry) => (
                    <Message from={entry.role} key={entry.id}>
                      <MessageContent
                        variant='flat'
                        className={
                          entry.error
                            ? 'text-destructive max-w-full min-w-0 gap-3 text-sm leading-6'
                            : 'max-w-full min-w-0 gap-3 text-sm leading-6'
                        }
                      >
                        {entry.role === 'assistant' ? (
                          <Response
                            className='max-w-full leading-7 break-words'
                            final
                          >
                            {entry.content}
                          </Response>
                        ) : (
                          <p className='break-words whitespace-pre-wrap'>
                            {entry.content}
                          </p>
                        )}
                        {entry.adminChange ? (
                          <AssistantAdminChangeTool
                            action={entry.adminChange}
                            onApplied={() => {
                              void statusQuery.refetch()
                            }}
                          />
                        ) : null}
                        {entry.retry || entry.action ? (
                          <div className='flex flex-wrap gap-2'>
                            {entry.retry ? (
                              <Button
                                type='button'
                                variant='outline'
                                onClick={() => void retryMessage(entry)}
                                disabled={sending}
                              >
                                <HugeiconsIcon
                                  icon={ReloadIcon}
                                  strokeWidth={2}
                                  data-icon='inline-start'
                                  aria-hidden='true'
                                />
                                {t('Retry')}
                              </Button>
                            ) : null}
                            {entry.action ? (
                              <AssistantActionButton
                                action={entry.action}
                                onToolOpen={setActiveTool}
                              />
                            ) : null}
                          </div>
                        ) : null}
                      </MessageContent>
                    </Message>
                  ))}
                  {sending ? (
                    <Message from='assistant'>
                      <MessageContent
                        variant='flat'
                        className='text-muted-foreground flex-row items-center gap-2'
                        aria-live='polite'
                      >
                        <Loader size={14} />
                        <span>{t('Assistant is thinking...')}</span>
                      </MessageContent>
                    </Message>
                  ) : null}
                  {accountToolActive && !accountAccessConfirmed ? (
                    <AssistantAccountStatusNotice
                      state={
                        accountAccessState === 'error' ? 'error' : 'loading'
                      }
                      onRetry={() => void statusQuery.refetch()}
                    />
                  ) : null}
                  {activeTool === 'key' && developerAccessGranted ? (
                    <AssistantKeyTool
                      baseUrl={baseUrl}
                      availableModels={connectionModelsQuery.data ?? []}
                      modelsLoading={connectionModelsQuery.isLoading}
                      developerAccessGranted={developerAccessGranted}
                      onKeyCreated={() => {
                        if (authUser) {
                          void queryClient.invalidateQueries({
                            queryKey: [
                              'assistant-onboarding-todo',
                              authUser.id,
                            ],
                          })
                        }
                      }}
                      onContinueSetup={() => setActiveTool('setup')}
                    />
                  ) : null}
                  {activeTool === 'activation' && accountAccessConfirmed ? (
                    <AssistantActivationTool
                      recommendationDraft={recommendationDraft}
                      onContinueSetup={() => setActiveTool('setup')}
                      onSubmitted={() => {
                        setRecommendationDraft(null)
                        setEntries((current) => [
                          ...current,
                          {
                            id: nanoid(),
                            role: 'assistant',
                            content: t(
                              'Your AI recommendation was sent to an administrator. L1 remains locked until the administrator approves it.'
                            ),
                          },
                        ])
                      }}
                    />
                  ) : null}
                  {accountDisableDraft ? (
                    <AssistantAccountActionTool
                      action={accountDisableDraft}
                      onSubmitted={() => setAccountDisableDraft(null)}
                    />
                  ) : null}
                  {activeTool === 'cost' && accountAccessConfirmed ? (
                    <AssistantCostTool
                      developerAccessGranted={developerAccessGranted}
                    />
                  ) : null}
                  {activeTool === 'handoff' && developerAccessGranted ? (
                    <AssistantHandoffTool />
                  ) : null}
                  {activeTool === 'models' && developerAccessGranted ? (
                    <AssistantModelsTool />
                  ) : null}
                  {activeTool === 'plan' && accountAccessConfirmed ? (
                    <AssistantPlanTool
                      developerAccessGranted={developerAccessGranted}
                      onRequestAccess={() => setActiveTool('activation')}
                    />
                  ) : null}
                  {activeTool === 'setup' && accountAccessConfirmed ? (
                    <AssistantSetupTool
                      rootUrl={baseUrl.replace(/\/v1$/, '')}
                      openAIBaseUrl={baseUrl}
                      availableModels={connectionModelsQuery.data ?? []}
                      modelsLoading={connectionModelsQuery.isLoading}
                      developerAccessGranted={developerAccessGranted}
                      onCreateKey={() => setActiveTool('key')}
                      onRequestAccess={() => setActiveTool('activation')}
                    />
                  ) : null}
                  {activeTool === 'usage' && developerAccessGranted ? (
                    <AssistantUsageTool
                      developerAccessGranted={developerAccessGranted}
                    />
                  ) : null}
                  <div className='grid gap-3 pt-1'>
                    <Separator />
                    <Button
                      type='button'
                      variant='ghost'
                      size='sm'
                      onClick={resetConversation}
                      disabled={sending}
                    >
                      <HugeiconsIcon
                        icon={CleanIcon}
                        strokeWidth={2}
                        data-icon='inline-start'
                        aria-hidden='true'
                      />
                      {t('Clear conversation')}
                    </Button>
                  </div>
                </>
              )}
            </ConversationContent>
            <ConversationScrollButton />
          </Conversation>

          <div className='bg-background min-w-0 shrink-0 pb-[max(0.75rem,env(safe-area-inset-bottom))]'>
            <Separator className='bg-border/70' />
            <div className='px-3 py-3 sm:px-4'>
              <PromptInputProvider
                key={conversationResetRevision}
                initialInput={props.initialMessage}
              >
                <AssistantPromptInputSync
                  initialMessage={props.initialMessage}
                  initialMessageRevision={props.initialMessageRevision}
                />
                <AssistantPromptComposer
                  footerStatus={assistantFooterStatus}
                  placeholder={assistantPromptPlaceholder}
                  privacyNoticeId='assistant-privacy-notice'
                  restricted={accountAccessState === 'restricted'}
                  sending={sending}
                  onSubmit={submitMessage}
                />
              </PromptInputProvider>
              {accountAccessConfirmed && superAdministratorFunded ? (
                <p className='text-muted-foreground mt-2 px-1 text-[11px] leading-4'>
                  {t(
                    'AI customer-service token usage is charged to the super administrator account, not your wallet.'
                  )}
                </p>
              ) : null}
              <p className='text-muted-foreground mt-1 px-1 text-[11px] leading-4'>
                {t(
                  'Private details, passwords, API keys, and credentials are never safe to send in chat. Use a shielded private card when one is offered.'
                )}
              </p>
            </div>
          </div>
        </>
      )}
    </>
  )

  if (mode === 'rail') {
    if (props.fullscreen) {
      return (
        <div
          id='ai-assistant-panel'
          role='dialog'
          aria-modal='true'
          aria-label={t('Service guide')}
          className='bg-background fixed inset-0 z-50 flex min-h-0 flex-col'
        >
          {panelContent}
        </div>
      )
    }
    if (props.collapsed) {
      return (
        <aside
          id='ai-assistant-panel'
          className='bg-background hidden min-h-0 w-12 shrink-0 flex-col border-l md:flex'
          aria-label={t('Service guide')}
        >
          <Button
            type='button'
            variant='ghost'
            size='icon'
            className='m-2'
            aria-label={t('Expand')}
            title={t('Expand')}
            data-testid='assistant-expand'
            onClick={props.onToggleCollapsed}
          >
            <HugeiconsIcon
              icon={ArrowRight01Icon}
              strokeWidth={2}
              aria-hidden='true'
            />
            <span className='sr-only'>{t('Expand')}</span>
          </Button>
          <span className='sr-only'>{t('Service guide')}</span>
        </aside>
      )
    }

    return (
      <aside
        id='ai-assistant-panel'
        className='bg-background hidden min-h-0 w-[min(28vw,30rem)] max-w-full min-w-0 shrink-0 flex-col border-l md:flex'
        aria-label={t('Service guide')}
      >
        {panelContent}
      </aside>
    )
  }

  return (
    <Sheet open={props.open} onOpenChange={handleOpenChange}>
      <SheetContent
        id='ai-assistant-panel'
        className={sideDrawerContentClassName(
          'inset-0 !h-dvh !min-h-dvh !w-screen !max-w-none !min-w-0 rounded-none'
        )}
      >
        {panelContent}
      </SheetContent>
    </Sheet>
  )
}
