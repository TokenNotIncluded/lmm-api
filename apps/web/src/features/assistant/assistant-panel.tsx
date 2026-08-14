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
import { PanelLeft, Plus } from 'lucide-react'
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
import { LmmBrandMark } from '@/components/lmm-brand-mark'
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

import {
  getAssistantAvailableModels,
  getAssistantPreConversationPresets,
  getAssistantStatus,
  recordAssistantPreConversationPresetClick,
  sendAssistantMessage,
  type AssistantPreConversationPreset,
  type AssistantConversationHistoryItem,
  type AssistantConversationHistoryDetail,
  type AssistantChatMessage,
  type AssistantAccountDisableAction,
  type AssistantCreateKeyAction,
  type AssistantAdminChangeAction,
  type AssistantImageGenerationAction,
  type AssistantHumanSupportAction,
  type AssistantL1RecommendationAction,
  type AssistantNavigationAction,
  type AssistantToolTrace,
  type AssistantUserAction,
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
import { AssistantImageTool } from './assistant-image-tool'
import {
  getExplicitAssistantNavigation,
  getAssistantPresetForIntent,
  isExplicitAssistantL1Request,
} from './assistant-intent'
import { AssistantJourneyProgress } from './assistant-journey'
import { AssistantKeyTool } from './assistant-key-tool'
import {
  hasAssistantMessageSubstantialMeaning,
  redactAssistantMessageForDisplay,
  redactAssistantMessageForRequest,
} from './assistant-message-safety'
import { AssistantModelsTool } from './assistant-models-tool'
import { AssistantNewUserGift } from './assistant-new-user-gift'
import { AssistantOnboardingTodo } from './assistant-onboarding-todo'
import { AssistantPlanTool } from './assistant-plan-tool'
import { getAssistantPromptValidation } from './assistant-prompt-validation'
import { AssistantSetupTool } from './assistant-setup-tool'
import { AssistantToolCalls } from './assistant-tool-calls'
import { AssistantUsageTool } from './assistant-usage-tool'
import { AssistantUserActionTool } from './assistant-user-action-tool'

type AssistantActionPath =
  | '/'
  | '/getting-started'
  | '/pricing'
  | '/wallet'
  | '/usage-logs'
  | '/keys'
  | '/drawing'
  | '/profile'
  | '/open-source-bounties'
  | '/support'
  | '/users'

type AssistantAction =
  | { kind: 'route'; label: string; to: AssistantActionPath }
  | { kind: 'navigation'; label: string; href: string }
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
  tools?: AssistantToolTrace[]
  action?: AssistantAction
  adminChange?: AssistantAdminChangeAction
  imageAction?: AssistantImageGenerationAction
  error?: boolean
  notice?: boolean
  retry?: {
    message: string
    history: AssistantChatMessage[]
    presetId?: string
  }
}

function isAssistantKeyConfirmationMessage(message: string) {
  const normalized = message.trim().toLowerCase()
  return /^(是|好|可以|确认|确认创建|创建|没问题|yes|y|ok|okay|confirm|go ahead|proceed)[！!。.]?$/.test(
    normalized
  )
}

function assistantNavigationHref(action: AssistantNavigationAction): string {
  const entries = Object.entries(action.query)
  if (entries.length === 0) return action.path
  const query = new URLSearchParams()
  for (const [key, value] of entries) query.set(key, String(value))
  return `${action.path}?${query.toString()}`
}

function assistantNavigationLabel(
  action: AssistantNavigationAction,
  translate: (key: string) => string
): string {
  if (action.path === '/users' && action.query.filter) {
    return translate('Locate user')
  }
  if (action.path.startsWith('/usage-logs')) {
    return translate('Open usage logs')
  }
  if (action.path === '/profile') return translate('Open account bindings')
  return translate('Open page')
}

type AssistantPanelMode = 'mobile' | 'page' | 'rail'

function getBaseUrl(): string {
  if (typeof window === 'undefined') return 'https://api.lmm.best/v1'
  return `${window.location.origin}/v1`
}

const ASSISTANT_PRIVACY_NOTICE_COLLAPSE_DELAY_MS = 5_000
const ASSISTANT_LAYOUT_STORAGE_KEY = 'lmm-assistant-layout'

function readAssistantClassicLayout(): boolean {
  if (typeof window === 'undefined') return false
  try {
    return (
      window.localStorage.getItem(ASSISTANT_LAYOUT_STORAGE_KEY) === 'classic'
    )
  } catch {
    return false
  }
}

function AssistantClassicWelcome() {
  const { t } = useTranslation()
  const columns = [
    {
      title: t('Examples'),
      items: [
        t('Explain an API setup'),
        t('Compare live model pricing'),
        t('Draft an access request'),
      ],
    },
    {
      title: t('Capabilities'),
      items: [
        t('Live models and pricing'),
        t('Step-by-step setup guidance'),
        t('Confirm sensitive actions yourself'),
      ],
    },
    {
      title: t('Limitations'),
      items: [
        t('Permissions still apply'),
        t('Never share secrets in chat'),
        t('Write actions need your confirmation'),
      ],
    },
  ]

  return (
    <div
      className='mx-auto grid w-full max-w-4xl gap-10 py-10 sm:grid-cols-3 sm:gap-8 sm:py-16'
      data-testid='assistant-classic-welcome'
    >
      {columns.map((column) => (
        <section key={column.title} className='space-y-3'>
          <h2 className='text-center text-sm font-semibold sm:text-base'>
            {column.title}
          </h2>
          <div className='space-y-2'>
            {column.items.map((item) => (
              <p
                key={item}
                className='bg-background/70 text-muted-foreground rounded-lg px-3 py-2.5 text-center text-sm leading-5 shadow-sm ring-1 ring-black/5 dark:ring-white/10'
              >
                {item}
              </p>
            ))}
          </div>
        </section>
      ))}
    </div>
  )
}

function AssistantClassicSidebar(props: {
  onNewConversation: () => void
  onOpenHistory: () => void
  onToggleLayout: () => void
}) {
  const { t } = useTranslation()
  return (
    <aside
      className='hidden w-64 shrink-0 flex-col bg-zinc-950 text-zinc-100 md:flex'
      data-testid='assistant-classic-sidebar'
    >
      <div className='flex items-center gap-3 px-4 py-5'>
        <LmmBrandMark className='size-8' />
        <div className='min-w-0'>
          <p className='truncate text-sm font-semibold'>LMM Assistant</p>
          <p className='truncate text-xs text-zinc-400'>{t('Service guide')}</p>
        </div>
      </div>
      <div className='px-3'>
        <Button
          type='button'
          variant='outline'
          className='w-full justify-start border-white/15 bg-white/5 text-zinc-100 hover:bg-white/10 hover:text-white'
          onClick={props.onNewConversation}
        >
          <Plus data-icon='inline-start' aria-hidden='true' />
          {t('New conversation')}
        </Button>
      </div>
      <nav
        className='mt-5 space-y-1 px-3'
        aria-label={t('Conversation history')}
      >
        <Button
          type='button'
          variant='ghost'
          className='w-full justify-start text-zinc-300 hover:bg-white/10 hover:text-white'
          onClick={props.onOpenHistory}
        >
          <PanelLeft data-icon='inline-start' aria-hidden='true' />
          {t('Conversation history')}
        </Button>
      </nav>
      <div className='mt-auto border-t border-white/10 px-3 py-4'>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='w-full justify-start text-zinc-400 hover:bg-white/10 hover:text-white'
          onClick={props.onToggleLayout}
        >
          <PanelLeft data-icon='inline-start' aria-hidden='true' />
          {t('Use modern layout')}
        </Button>
      </div>
    </aside>
  )
}

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

  if (action.kind === 'navigation') {
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

function AssistantPresetPrompts(props: {
  presets: AssistantPreConversationPreset[]
  onSelect: (preset: AssistantPreConversationPreset) => void
}) {
  const { t } = useTranslation()
  const {
    textInput: { setInput },
  } = usePromptInputController()
  if (props.presets.length === 0) return null

  return (
    <div
      className='mb-2 flex max-w-full flex-nowrap gap-2 overflow-x-auto pb-1 sm:flex-wrap sm:overflow-visible sm:pb-0'
      role='group'
      aria-label={t('Choose a topic or write a message.')}
      data-testid='assistant-preset-prompts'
    >
      {props.presets.map((preset) => (
        <Button
          key={preset.id}
          type='button'
          variant='ghost'
          size='sm'
          className='bg-muted/40 h-auto min-h-8 shrink-0 rounded-full px-3 text-left whitespace-normal'
          onClick={() => {
            setInput(preset.prompt)
            props.onSelect(preset)
          }}
        >
          {preset.label || preset.prompt}
        </Button>
      ))}
    </div>
  )
}

function AssistantPromptComposer(props: {
  footerStatus: string
  placeholder: string
  privacyNoticeId: string
  classicLayout?: boolean
  restricted: boolean
  terminated: boolean
  sending: boolean
  onSubmit: (message: { text?: string }) => void | Promise<void>
}) {
  const { t } = useTranslation()
  const {
    textInput: { setInput, value },
  } = usePromptInputController()
  const onSubmit = props.onSubmit
  const validation = getAssistantPromptValidation(value, props.restricted)
  const hasText = value.trim().length > 0
  const showValidationError = hasText && validation.invalid
  const hintId = 'assistant-l0-input-hint'
  const describedBy = showValidationError
    ? `${props.privacyNoticeId} ${hintId}`
    : props.privacyNoticeId

  const handleSubmit = useCallback(
    (message: { text?: string }) => {
      const safeMessage = redactAssistantMessageForRequest(message.text ?? '')
      // PromptInput keeps provider state until the async submit resolves. Set
      // the safe value immediately so the raw textarea value cannot linger
      // while the assistant request is in flight.
      if (safeMessage.redacted) setInput(safeMessage.content)
      return onSubmit({ text: safeMessage.content })
    },
    [onSubmit, setInput]
  )

  return (
    <>
      <PromptInput
        onSubmit={handleSubmit}
        groupClassName={cn(
          'assistant-prompt-input has-[[data-slot=input-group-control]:focus-visible]:border-transparent has-[[data-slot=input-group-control]:focus-visible]:ring-0 rounded-xl border-transparent',
          props.classicLayout
            ? 'bg-background shadow-sm ring-1 ring-black/10 dark:bg-zinc-900 dark:ring-white/10'
            : 'bg-muted/40 dark:bg-muted/30'
        )}
        aria-label={t('Ask AI assistant')}
        data-testid='assistant-prompt-form'
      >
        <PromptInputBody>
          <PromptInputTextarea
            placeholder={
              props.terminated
                ? t('This conversation has ended. Start a new conversation.')
                : props.placeholder
            }
            aria-label={t('Ask AI assistant')}
            maxLength={4000}
            required={props.restricted}
            aria-describedby={describedBy}
            aria-invalid={hasText ? validation.invalid : undefined}
            disabled={props.sending || props.terminated}
            className='max-h-24 min-h-10 sm:max-h-32 sm:min-h-12'
          />
        </PromptInputBody>
        <PromptInputFooter className='px-2 py-1 pb-1.5'>
          <span className='text-muted-foreground min-w-0 flex-1 truncate text-xs'>
            {props.footerStatus}
          </span>
          <PromptInputSubmit
            status={props.sending ? 'submitted' : 'ready'}
            disabled={props.sending || props.terminated || validation.invalid}
            size='sm'
          >
            {t('Send')}
          </PromptInputSubmit>
        </PromptInputFooter>
      </PromptInput>
      {showValidationError ? (
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
          {t('Please enter a message other than a single punctuation mark.')}
        </p>
      ) : null}
    </>
  )
}

function AssistantPanelHeader(props: {
  mode: AssistantPanelMode
  description: string
  classicLayout: boolean
  onToggleClassicLayout: () => void
  historyVisible: boolean
  historyDetail: boolean
  onOpenHistory: () => void
  onCloseHistory: () => void
  onToggleCollapsed?: () => void
  fullscreen?: boolean
  onToggleFullscreen?: () => void
}) {
  const { t } = useTranslation()

  if (props.mode === 'page') {
    return (
      <header
        className={cn(
          'assistant-glass-surface border-border/60 flex min-w-0 shrink-0 flex-wrap items-center gap-3 border-b px-4 py-3 sm:px-6',
          props.classicLayout ? 'bg-background/90' : 'bg-background/70'
        )}
      >
        <h1 className='min-w-0 flex-1 truncate text-sm font-medium'>
          {props.classicLayout ? 'LMM Assistant' : t('Service guide')}
        </h1>
        <AssistantJourneyProgress presentation='page' />
        <Button
          type='button'
          variant='ghost'
          size='sm'
          aria-pressed={props.classicLayout}
          aria-label={
            props.classicLayout
              ? t('Use modern layout')
              : t('Use classic layout')
          }
          data-testid='assistant-layout-toggle'
          onClick={props.onToggleClassicLayout}
        >
          <PanelLeft data-icon='inline-start' aria-hidden='true' />
          <span className='hidden sm:inline'>
            {props.classicLayout ? t('Modern chat') : t('Classic chat')}
          </span>
        </Button>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          onClick={
            props.historyVisible ? props.onCloseHistory : props.onOpenHistory
          }
        >
          {props.historyVisible
            ? props.historyDetail
              ? t('Conversation history')
              : t('Back to conversation')
            : t('Conversation history')}
        </Button>
      </header>
    )
  }

  if (props.mode === 'mobile') {
    return (
      <SheetHeader
        className={sideDrawerHeaderClassName(
          'min-w-0 shrink-0 pr-12 pt-[max(0.75rem,env(safe-area-inset-top))]'
        )}
      >
        <SheetTitle>{t('Service guide')}</SheetTitle>
        <SheetDescription className='min-w-0 break-words'>
          {props.description}
        </SheetDescription>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='mt-2 self-start'
          onClick={
            props.historyVisible ? props.onCloseHistory : props.onOpenHistory
          }
        >
          {props.historyVisible
            ? props.historyDetail
              ? t('Conversation history')
              : t('Back to conversation')
            : t('Conversation history')}
        </Button>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='mt-1 self-start'
          aria-pressed={props.classicLayout}
          data-testid='assistant-layout-toggle'
          onClick={props.onToggleClassicLayout}
        >
          <PanelLeft data-icon='inline-start' aria-hidden='true' />
          {props.classicLayout ? t('Modern chat') : t('Classic chat')}
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
          className='hidden max-w-28 truncate px-2 sm:inline-flex'
          aria-pressed={props.classicLayout}
          data-testid='assistant-layout-toggle'
          onClick={props.onToggleClassicLayout}
        >
          {props.classicLayout ? t('Modern chat') : t('Classic chat')}
        </Button>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='max-w-32 shrink-0 truncate px-2 sm:max-w-40'
          onClick={
            props.historyVisible ? props.onCloseHistory : props.onOpenHistory
          }
        >
          {props.historyVisible
            ? props.historyDetail
              ? t('Conversation history')
              : t('Back')
            : t('Conversation history')}
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
  autoSendRequestId?: string
  onAutoSendConsumed?: (requestId: string) => void
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
    mode === 'page'
      ? true
      : mode === 'rail'
        ? !props.collapsed || props.fullscreen === true
        : props.open
  const baseUrl = getBaseUrl()
  const [entries, setEntries] = useState<ConversationEntry[]>([])
  const [conversationId, setConversationId] = useState<number | null>(null)
  const [selectedPreConversationPresetId, setSelectedPreConversationPresetId] =
    useState<string | null>(null)
  const [conversationRestricted, setConversationRestricted] = useState(false)
  const [sending, setSending] = useState(false)
  const [classicLayout, setClassicLayout] = useState(readAssistantClassicLayout)
  const submittedAutoSendIdRef = useRef<string | undefined>(undefined)
  const [recommendationDraft, setRecommendationDraft] =
    useState<AssistantL1RecommendationAction | null>(null)
  const [accountDisableDraft, setAccountDisableDraft] =
    useState<AssistantAccountDisableAction | null>(null)
  const [humanSupportAction, setHumanSupportAction] =
    useState<AssistantHumanSupportAction | null>(null)
  const [keyCreationAction, setKeyCreationAction] =
    useState<AssistantCreateKeyAction | null>(null)
  const [autoConfirmKeyToken, setAutoConfirmKeyToken] = useState<string | null>(
    null
  )
  const [userActionDraft, setUserActionDraft] =
    useState<AssistantUserAction | null>(null)
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
  const activeToolRegionRef = useRef<HTMLDivElement | null>(null)
  const [conversationResetRevision, setConversationResetRevision] = useState(0)
  const [privacyNoticeExpanded, setPrivacyNoticeExpanded] = useState(
    mode !== 'page'
  )
  const privacyNoticeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null
  )
  useEffect(() => {
    try {
      window.localStorage.setItem(
        ASSISTANT_LAYOUT_STORAGE_KEY,
        classicLayout ? 'classic' : 'modern'
      )
    } catch {
      // A private browsing context may deny storage; the in-memory toggle still works.
    }
  }, [classicLayout])
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
  const preConversationPresetsQuery = useQuery({
    queryKey: ['assistant-pre-conversation-presets'],
    queryFn: getAssistantPreConversationPresets,
    enabled: panelVisible && accountAccessState === 'restricted',
    staleTime: 5 * 60_000,
    retry: false,
  })
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
    if (!panelVisible || mode === 'page') {
      clearPrivacyNoticeTimer()
      return
    }

    schedulePrivacyNoticeCollapse()
    return clearPrivacyNoticeTimer
  }, [
    clearPrivacyNoticeTimer,
    mode,
    panelVisible,
    schedulePrivacyNoticeCollapse,
  ])
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

  const openAssistantTool = useCallback((tool: AssistantTool) => {
    setActiveTool(tool)
    window.requestAnimationFrame(() => {
      activeToolRegionRef.current?.focus({ preventScroll: true })
      activeToolRegionRef.current?.scrollIntoView({
        behavior: 'smooth',
        block: 'nearest',
      })
    })
  }, [])

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
      'Guidance for plans, setup, API keys, costs, and support.'
    )
    assistantPromptPlaceholder = t('Ask AI assistant')
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
    setHumanSupportAction(null)
    setKeyCreationAction(null)
    setAutoConfirmKeyToken(null)
    setUserActionDraft(null)
  }, [])

  const clearTransientCards = useCallback(() => {
    clearToolState()
    setEntries((current) =>
      current.map((entry) =>
        entry.action || entry.adminChange || entry.imageAction
          ? {
              ...entry,
              action: undefined,
              adminChange: undefined,
              imageAction: undefined,
            }
          : entry
      )
    )
  }, [clearToolState])

  const resetConversation = useCallback(() => {
    setEntries([])
    setConversationId(null)
    setSelectedPreConversationPresetId(null)
    setConversationRestricted(false)
    clearToolState()
    setHistoryView(null)
    openedTargetRef.current = undefined
    setConversationResetRevision((revision) => revision + 1)
    onConversationReset?.()
  }, [clearToolState, onConversationReset])

  const continueHistoryConversation = useCallback(
    (detail: AssistantConversationHistoryDetail) => {
      const restoredEntries = detail.messages.flatMap<ConversationEntry>(
        (message) => {
          if (message.role !== 'user' && message.role !== 'assistant') return []
          const safeMessage = redactAssistantMessageForDisplay(
            message.content,
            t(
              'Sensitive content is hidden and can only be accessed from a private card.'
            )
          )
          return [
            {
              id: `history-${message.id}`,
              role: message.role,
              content: safeMessage.content,
            },
          ]
        }
      )
      setEntries(restoredEntries)
      setConversationId(detail.conversation.id)
      setConversationRestricted(Boolean(detail.conversation.restricted_at))
      clearToolState()
      setHistoryView(null)
    },
    [clearToolState, t]
  )

  const openAssistantTarget = useCallback(
    (target: AssistantPresetId) => {
      const restrictedTarget =
        target === 'onboarding' ||
        target === 'client-setup' ||
        target === 'bounty' ||
        target === 'cost' ||
        target === 'human'
      if (
        accountAccessState !== 'granted' &&
        !(accountAccessState === 'restricted' && restrictedTarget)
      ) {
        return false
      }

      const tool = getAssistantToolForTarget(target)
      const action = getAssistantActionForTarget(target, t)
      clearTransientCards()
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
    [accountAccessState, assistantDescription, clearTransientCards, t]
  )

  useEffect(() => {
    const target = props.initialPreset
    if (!target || !accountAccessConfirmed) return
    if (openedTargetRef.current === target) return
    if (openAssistantTarget(target)) openedTargetRef.current = target
  }, [accountAccessConfirmed, openAssistantTarget, props.initialPreset])

  useEffect(
    () =>
      subscribeToAssistantOpen((request) => {
        const target = request.preset
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
    history: AssistantChatMessage[],
    presetId?: string
  ) => {
    setSending(true)
    try {
      const reply = await sendAssistantMessage(
        message,
        history,
        conversationId ?? undefined,
        presetId
      )
      if (reply.conversationId) setConversationId(reply.conversationId)
      if (reply.restricted) setConversationRestricted(true)
      const safeReply = redactAssistantMessageForDisplay(
        reply.content,
        t(
          'Sensitive content is hidden and can only be accessed from a private card.'
        )
      )
      const suggestedTarget = getAssistantPresetForIntent(reply.intent)
      const explicitNavigation = getExplicitAssistantNavigation(
        message,
        reply.intent
      )
      const adminChange =
        reply.action?.type === 'admin_config_change' ||
        reply.action?.type === 'admin_pricing_change' ||
        reply.action?.type === 'admin_model_sync'
          ? reply.action
          : undefined
      const imageAction =
        reply.action?.type === 'image_generation' ? reply.action : undefined
      const humanSupportAction =
        reply.action?.type === 'human_support' ? reply.action : undefined
      const userAction =
        reply.action?.type === 'user_password_change' ||
        reply.action?.type === 'user_oauth_unbind' ||
        reply.action?.type === 'user_account_action'
          ? reply.action
          : undefined
      let suggestedAction: AssistantAction | undefined
      const restrictedTargetAllowed =
        accountAccessState === 'restricted' &&
        suggestedTarget !== undefined &&
        (suggestedTarget !== 'onboarding' ||
          isExplicitAssistantL1Request(message)) &&
        [
          'onboarding',
          'client-setup',
          'cost',
          'bounty',
          'plan',
          'human',
        ].includes(suggestedTarget)
      if (developerAccessGranted || restrictedTargetAllowed) {
        suggestedAction = getAssistantActionForTarget(suggestedTarget, t)
      }
      if (imageAction) {
        setRecommendationDraft(null)
        setAccountDisableDraft(null)
        setHumanSupportAction(null)
        setKeyCreationAction(null)
        setUserActionDraft(null)
        setActiveTool(null)
        suggestedAction = undefined
      } else if (adminChange) {
        setRecommendationDraft(null)
        setAccountDisableDraft(null)
        setHumanSupportAction(null)
        setKeyCreationAction(null)
        setUserActionDraft(null)
        setActiveTool(null)
        suggestedAction = undefined
      } else if (reply.action?.type === 'navigate') {
        setRecommendationDraft(null)
        setAccountDisableDraft(null)
        setHumanSupportAction(null)
        setUserActionDraft(null)
        setActiveTool(null)
        suggestedAction = {
          kind: 'navigation',
          label: assistantNavigationLabel(reply.action, t),
          href: assistantNavigationHref(reply.action),
        }
      } else if (reply.action?.type === 'l1_recommendation') {
        setRecommendationDraft(reply.action)
        setAccountDisableDraft(null)
        setHumanSupportAction(null)
        setUserActionDraft(null)
        setActiveTool('activation')
        suggestedAction = {
          kind: 'tool',
          label: t('Review AI recommendation'),
          tool: 'activation',
        }
      } else if (reply.action?.type === 'account_disable_request') {
        setAccountDisableDraft(reply.action)
        setHumanSupportAction(null)
        setRecommendationDraft(null)
        setKeyCreationAction(null)
        setUserActionDraft(null)
        setActiveTool(null)
        suggestedAction = undefined
      } else if (reply.action?.type === 'create_key') {
        setKeyCreationAction(reply.action)
        setHumanSupportAction(null)
        setRecommendationDraft(null)
        setAccountDisableDraft(null)
        setUserActionDraft(null)
        setActiveTool('key')
        suggestedAction = undefined
      } else if (userAction) {
        setRecommendationDraft(null)
        setAccountDisableDraft(null)
        setHumanSupportAction(null)
        setKeyCreationAction(null)
        setUserActionDraft(userAction)
        setActiveTool(null)
        suggestedAction = undefined
      } else if (humanSupportAction) {
        setRecommendationDraft(null)
        setAccountDisableDraft(null)
        setHumanSupportAction(humanSupportAction)
        setKeyCreationAction(null)
        setUserActionDraft(null)
        setActiveTool('handoff')
        suggestedAction = undefined
      }
      if (
        accountAccessState === 'restricted' &&
        isExplicitAssistantL1Request(message) &&
        !adminChange &&
        !imageAction &&
        !humanSupportAction &&
        !userAction &&
        reply.action?.type !== 'account_disable_request' &&
        reply.action?.type !== 'navigate' &&
        reply.action?.type !== 'create_key' &&
        reply.action?.type !== 'human_support'
      ) {
        setActiveTool('activation')
        suggestedAction ??= {
          kind: 'tool',
          label: t('Submit for administrator review'),
          tool: 'activation',
        }
      }
      setEntries((current) => [
        ...current,
        {
          id: nanoid(),
          role: 'assistant',
          content: safeReply.content,
          tools: reply.tools,
          action: suggestedAction,
          adminChange,
          imageAction,
        },
      ])
      if (explicitNavigation && developerAccessGranted) {
        void navigate({ to: explicitNavigation })
      }
      await queryClient.invalidateQueries({ queryKey: ['assistant-status'] })
      await queryClient.invalidateQueries({ queryKey: ['assistant-journey'] })
      await queryClient.invalidateQueries({
        queryKey: ['assistant-new-user-gift'],
      })
      await queryClient.invalidateQueries({
        queryKey: ['assistant-conversations'],
      })
    } catch {
      const canSubmitWithoutAssistant =
        accountAccessState === 'restricted' &&
        isExplicitAssistantL1Request(message)
      if (canSubmitWithoutAssistant) {
        setRecommendationDraft(null)
        setActiveTool('activation')
      }
      let errorAction: AssistantAction | undefined
      if (canSubmitWithoutAssistant) {
        errorAction = {
          kind: 'tool',
          label: t('Submit for administrator review'),
          tool: 'activation',
        }
      } else if (developerAccessGranted) {
        errorAction = {
          kind: 'route',
          label: t('Contact support'),
          to: '/support',
        }
      }
      setEntries((current) => [
        ...current,
        {
          id: nanoid(),
          role: 'assistant',
          content: t(
            'The AI assistant could not answer right now. Try again or contact support.'
          ),
          error: true,
          retry: { message, history, presetId },
          action: errorAction,
        },
      ])
    } finally {
      setSending(false)
    }
  }

  const submitMessage = async ({ text }: { text?: string }) => {
    const message = text?.trim()
    if (sending || conversationRestricted) return
    if (!message) {
      throw new Error(t('Please enter a message.'))
    }
    const validation = getAssistantPromptValidation(message)
    if (validation.invalid) {
      throw new Error(
        t('Please enter a message other than a single punctuation mark.')
      )
    }
    const pendingKeyConfirmation =
      keyCreationAction !== null && isAssistantKeyConfirmationMessage(message)
    if (!pendingKeyConfirmation) clearTransientCards()
    const safeMessage = redactAssistantMessageForRequest(message)
    if (!hasAssistantMessageSubstantialMeaning(safeMessage.content)) {
      setEntries((current) => [
        ...current,
        {
          id: nanoid(),
          role: 'assistant',
          content: t(
            'Only sensitive content remained after redaction. Add a question without including the secret.'
          ),
          notice: true,
        },
      ])
      return
    }
    const history: AssistantChatMessage[] = entries
      .filter((entry) => !entry.error && !entry.notice)
      .map((entry) => ({ role: entry.role, content: entry.content }))
    setEntries((current) => [
      ...current,
      { id: nanoid(), role: 'user', content: safeMessage.content },
      ...(safeMessage.redacted
        ? [
            {
              id: nanoid(),
              role: 'assistant' as const,
              content: t('Sensitive content was redacted before sending.'),
              notice: true,
            },
          ]
        : []),
    ])
    if (pendingKeyConfirmation && keyCreationAction) {
      setAutoConfirmKeyToken(keyCreationAction.confirmation_token)
      setActiveTool('key')
      setSelectedPreConversationPresetId(null)
      return
    }
    const presetId = selectedPreConversationPresetId ?? undefined
    setSelectedPreConversationPresetId(null)
    await requestAssistantReply(safeMessage.content, history, presetId)
  }

  useEffect(() => {
    const requestId = props.autoSendRequestId
    const message = props.initialMessage?.trim()
    if (
      !requestId ||
      !message ||
      !accountAccessConfirmed ||
      submittedAutoSendIdRef.current === requestId
    ) {
      return
    }
    submittedAutoSendIdRef.current = requestId
    props.onAutoSendConsumed?.(requestId)
    void submitMessage({ text: message })
    // The request id is a single-use delivery token. Keeping it in a ref
    // prevents React StrictMode from submitting it again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    accountAccessConfirmed,
    props.autoSendRequestId,
    props.initialMessage,
    props.onAutoSendConsumed,
  ])

  const retryMessage = async (entry: ConversationEntry) => {
    if (!entry.retry || sending) return
    setEntries((current) => current.filter((item) => item.id !== entry.id))
    await requestAssistantReply(
      entry.retry.message,
      entry.retry.history,
      entry.retry.presetId
    )
  }

  const panelContent = (
    <>
      <AssistantPanelHeader
        mode={mode}
        description={assistantDescription}
        classicLayout={classicLayout}
        onToggleClassicLayout={() => setClassicLayout((value) => !value)}
        historyVisible={historyVisible}
        historyDetail={historyView !== null && historyView !== 'list'}
        onOpenHistory={() => setHistoryView('list')}
        onCloseHistory={() =>
          setHistoryView((current) =>
            current !== null && current !== 'list' ? 'list' : null
          )
        }
        onToggleCollapsed={props.onToggleCollapsed}
        fullscreen={props.fullscreen}
        onToggleFullscreen={props.onToggleFullscreen}
      />
      <Alert
        id='assistant-privacy-notice'
        className={cn(
          'mb-0 min-w-0 overflow-hidden',
          mode === 'page' ? 'hidden' : 'm-3 max-w-[calc(100%-1.5rem)]',
          !privacyNoticeExpanded && 'py-1.5'
        )}
        data-testid='assistant-privacy-notice'
        variant='default'
      >
        <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} aria-hidden='true' />
        <div className='min-w-0'>
          <AlertTitle className='min-w-0'>
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
            <p className='break-words'>
              {t(
                'Your assistant conversations are not private. Authorized higher-access users may review them.'
              )}
            </p>
            <p className='break-words'>
              {t(
                'Do not send personal information, passwords, API keys, or credentials in chat. Site-issued credentials such as API keys are shown in a shielded private card and are kept out of the assistant context.'
              )}
            </p>
            <p className='break-words'>
              {t(
                'If you accidentally send supported sensitive data, the assistant safety filter may detect common email addresses, phone numbers, and API key formats and redact the message before this assistant request is sent. Pattern matching is not a guarantee.'
              )}
            </p>
          </AlertDescription>
        </div>
      </Alert>
      {historyVisible ? (
        <Conversation className='bg-muted/20 min-h-0 min-w-0 flex-1'>
          <ConversationContent
            className={cn(
              'flex min-h-full min-w-0 flex-col gap-5 overflow-x-hidden px-4 py-5 sm:px-6',
              mode === 'page' ? 'mx-auto w-full max-w-3xl' : 'max-w-full'
            )}
          >
            {historyView === 'list' ? (
              <AssistantHistory
                active={panelVisible}
                presentation='rows'
                onOpenConversation={(conversation) =>
                  setHistoryView(conversation)
                }
              />
            ) : (
              <AssistantHistoryConversation
                conversation={historyView}
                onContinue={continueHistoryConversation}
              />
            )}
          </ConversationContent>
        </Conversation>
      ) : (
        <>
          {mode !== 'page' && developerAccessGranted && authUser ? (
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
          <Conversation className='bg-muted/20 min-h-0 min-w-0 flex-1'>
            <ConversationContent
              className={cn(
                'flex min-h-full min-w-0 flex-col gap-5 overflow-x-hidden px-4 py-5 sm:px-6',
                classicLayout && mode === 'page' && 'gap-0 px-0 sm:px-0',
                mode === 'page' ? 'mx-auto w-full max-w-3xl' : 'max-w-full'
              )}
            >
              {entries.length === 0 ? (
                <div
                  className={cn(
                    'flex flex-1 flex-col gap-5',
                    mode === 'page' && 'justify-center py-12'
                  )}
                >
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
                    <div
                      className='space-y-2'
                      data-testid='assistant-l0-welcome'
                    >
                      <p className='text-base font-medium'>
                        {t('What would you like to do?')}
                      </p>
                      <p className='text-muted-foreground text-sm leading-6'>
                        {assistantDescription}
                      </p>
                    </div>
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
                  {classicLayout ? <AssistantClassicWelcome /> : null}
                </div>
              ) : (
                <>
                  {entries.map((entry) => (
                    <Message
                      from={entry.role}
                      key={entry.id}
                      className={cn(
                        classicLayout &&
                          mode === 'page' &&
                          'items-start px-4 py-4 sm:px-8',
                        classicLayout &&
                          mode === 'page' &&
                          entry.role === 'user' &&
                          'bg-background/70'
                      )}
                    >
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
                            className='max-w-full leading-7 break-words [&_pre]:max-w-full [&_pre]:overflow-x-auto'
                            final
                          >
                            {entry.content}
                          </Response>
                        ) : (
                          <p className='break-words whitespace-pre-wrap'>
                            {entry.content}
                          </p>
                        )}
                        {entry.tools?.length ? (
                          <AssistantToolCalls traces={entry.tools} />
                        ) : null}
                        {entry.imageAction ? (
                          <AssistantImageTool action={entry.imageAction} />
                        ) : null}
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
                                onToolOpen={openAssistantTool}
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
                  <AssistantNewUserGift enabled={accountAccessConfirmed} />
                  <div
                    ref={activeToolRegionRef}
                    className='grid gap-5 outline-none'
                    data-testid='assistant-active-tool-region'
                    tabIndex={-1}
                  >
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
                        confirmationAction={keyCreationAction}
                        autoConfirm={
                          autoConfirmKeyToken ===
                          keyCreationAction?.confirmation_token
                        }
                        onKeyCreated={() => {
                          setAutoConfirmKeyToken(null)
                          setKeyCreationAction(null)
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
                        onDraftConsumed={() => setRecommendationDraft(null)}
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
                    {userActionDraft ? (
                      <AssistantUserActionTool
                        action={userActionDraft}
                        onCompleted={() => setUserActionDraft(null)}
                      />
                    ) : null}
                    {activeTool === 'cost' && accountAccessConfirmed ? (
                      <AssistantCostTool
                        developerAccessGranted={developerAccessGranted}
                      />
                    ) : null}
                    {activeTool === 'handoff' && accountAccessConfirmed ? (
                      <AssistantHandoffTool
                        confirmationAction={humanSupportAction}
                      />
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
                  </div>

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

          <div
            className='bg-background min-w-0 shrink-0 overflow-hidden pb-[max(0.5rem,env(safe-area-inset-bottom))] sm:pb-[max(0.75rem,env(safe-area-inset-bottom))]'
            data-testid='assistant-composer-footer'
          >
            <Separator className='bg-border/70' />
            <div
              className={cn(
                'px-3 py-2 sm:px-4 sm:py-3',
                mode === 'page' && 'mx-auto w-full max-w-3xl'
              )}
            >
              <PromptInputProvider
                key={conversationResetRevision}
                initialInput={props.initialMessage}
              >
                <AssistantPromptInputSync
                  initialMessage={props.initialMessage}
                  initialMessageRevision={props.initialMessageRevision}
                />
                {accountAccessState === 'restricted' &&
                !entries.some((entry) => entry.role === 'user') ? (
                  <AssistantPresetPrompts
                    presets={preConversationPresetsQuery.data?.presets ?? []}
                    onSelect={(preset) => {
                      setSelectedPreConversationPresetId(preset.id)
                      void recordAssistantPreConversationPresetClick(
                        preset.id
                      ).catch(() => undefined)
                    }}
                  />
                ) : null}
                <AssistantPromptComposer
                  footerStatus={
                    conversationRestricted
                      ? t('Conversation ended by safety policy')
                      : assistantFooterStatus
                  }
                  placeholder={assistantPromptPlaceholder}
                  privacyNoticeId='assistant-privacy-notice'
                  classicLayout={classicLayout}
                  restricted={accountAccessState === 'restricted'}
                  terminated={conversationRestricted}
                  sending={sending}
                  onSubmit={submitMessage}
                />
              </PromptInputProvider>
            </div>
          </div>
        </>
      )}
    </>
  )

  if (mode === 'page') {
    return (
      <section
        id='ai-assistant-panel'
        className={cn(
          'bg-background flex min-h-0 min-w-0 flex-1',
          classicLayout && 'bg-muted/20'
        )}
        data-layout={classicLayout ? 'classic' : 'modern'}
        aria-label={t('Service guide')}
      >
        {classicLayout ? (
          <AssistantClassicSidebar
            onNewConversation={resetConversation}
            onOpenHistory={() => setHistoryView('list')}
            onToggleLayout={() => setClassicLayout(false)}
          />
        ) : null}
        <main className='flex min-h-0 min-w-0 flex-1 flex-col'>
          {panelContent}
        </main>
      </section>
    )
  }

  if (mode === 'rail') {
    if (props.fullscreen) {
      return (
        <div
          id='ai-assistant-panel'
          role='dialog'
          aria-modal='true'
          aria-label={t('Service guide')}
          className={cn(
            'bg-background fixed inset-0 z-50 flex min-h-0 flex-col',
            classicLayout && 'bg-muted/20'
          )}
          data-layout={classicLayout ? 'classic' : 'modern'}
        >
          {panelContent}
        </div>
      )
    }
    if (props.collapsed) {
      return (
        <aside
          id='ai-assistant-panel'
          className={cn(
            'bg-background hidden min-h-0 w-12 shrink-0 flex-col border-l xl:flex',
            classicLayout && 'bg-muted/20'
          )}
          data-layout={classicLayout ? 'classic' : 'modern'}
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
        className={cn(
          'bg-background hidden min-h-0 w-[min(28vw,30rem)] max-w-full min-w-0 shrink-0 flex-col border-l xl:flex',
          classicLayout && 'bg-muted/20'
        )}
        data-layout={classicLayout ? 'classic' : 'modern'}
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
          cn(
            'inset-0 !h-dvh !max-h-dvh !min-h-0 !w-screen !max-w-none !min-w-0 rounded-none overscroll-contain',
            classicLayout && 'bg-muted/20'
          )
        )}
        data-layout={classicLayout ? 'classic' : 'modern'}
      >
        {panelContent}
      </SheetContent>
    </Sheet>
  )
}
