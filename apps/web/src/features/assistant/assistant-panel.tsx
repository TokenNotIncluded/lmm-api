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
import { Link } from '@tanstack/react-router'
import { nanoid } from 'nanoid'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
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
import { AssistantPlanTool } from './assistant-plan-tool'
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

type AssistantPreset = {
  id: AssistantPresetId
  question: string
  answer: string
  action?: AssistantAction
  restricted?: {
    answer: string
    action?: AssistantAction
  }
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

function PresetAction(props: {
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

const L0_MINIMUM_MESSAGE_CHARACTERS = 5

export function getAssistantPromptValidation(
  message: string,
  restricted: boolean
) {
  const characterCount = Array.from(message.trim()).length
  return {
    characterCount,
    invalid: restricted && characterCount < L0_MINIMUM_MESSAGE_CHARACTERS,
  }
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
  const showValidationError = props.restricted && hasText && validation.invalid
  const hintId = 'assistant-l0-input-hint'
  const describedBy = props.restricted
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
            minLength={
              props.restricted ? L0_MINIMUM_MESSAGE_CHARACTERS : undefined
            }
            required={props.restricted}
            aria-describedby={describedBy}
            aria-invalid={
              props.restricted && hasText ? validation.invalid : undefined
            }
            disabled={props.sending}
            className='max-h-32 min-h-12'
          />
        </PromptInputBody>
        <PromptInputFooter>
          <span className='text-muted-foreground min-w-0 flex-1 truncate text-xs'>
            {props.footerStatus}
          </span>
          {props.restricted ? (
            <span
              className={
                showValidationError
                  ? 'text-destructive shrink-0 text-xs'
                  : 'text-muted-foreground shrink-0 text-xs'
              }
              aria-label={t('L0 message character count')}
            >
              {validation.characterCount}/{L0_MINIMUM_MESSAGE_CHARACTERS}
            </span>
          ) : null}
          <PromptInputSubmit
            status={props.sending ? 'submitted' : 'ready'}
            disabled={props.sending || validation.invalid}
          />
        </PromptInputFooter>
      </PromptInput>
      {props.restricted ? (
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
            ? t('Support message must contain at least 5 characters.')
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
      <SheetHeader className={sideDrawerHeaderClassName('pr-12')}>
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
    <header className='border-border/70 bg-background/95 supports-[backdrop-filter]:bg-background/80 flex shrink-0 items-start gap-3 border-b px-4 py-3 backdrop-blur'>
      <div className='min-w-0 flex-1'>
        <h2 className='text-base leading-6 font-semibold'>
          {t('Service guide')}
        </h2>
        <p className='text-muted-foreground mt-0.5 text-xs leading-5'>
          {props.description}
        </p>
      </div>
      <Button
        type='button'
        variant='ghost'
        size='sm'
        className='shrink-0'
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
  onToggleCollapsed?: () => void
  onToggleFullscreen?: () => void
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const mode = props.mode ?? 'mobile'
  const panelVisible =
    mode === 'rail' ? !props.collapsed || props.fullscreen === true : props.open
  const baseUrl = getBaseUrl()
  const presets = useMemo<AssistantPreset[]>(
    () => [
      {
        id: 'onboarding',
        question: t('Ask an administrator to raise my access level'),
        answer: t(
          'L0 accounts can browse challenges and ask the AI assistant to request L1 access.'
        ),
        action: {
          kind: 'tool',
          label: t('Unlock L1 access'),
          tool: 'activation',
        },
        restricted: {
          answer: t(
            'L0 accounts can browse challenges and ask the AI assistant to request L1 access.'
          ),
          action: {
            kind: 'tool',
            label: t('Unlock L1 access'),
            tool: 'activation',
          },
        },
      },
      {
        id: 'service',
        question: t('What can I do while access is under review?'),
        answer: t(
          'I can explain LMM services, compare plans, estimate costs, prepare client setup, and introduce open-source challenges while your request is reviewed.'
        ),
        restricted: {
          answer: t(
            'L0 access is restricted. Plans and top-up discounts stay hidden; explain your real use case and I can prepare an L1 recommendation for your confirmation.'
          ),
        },
      },
      {
        id: 'plan',
        question: t('Which option is the best value?'),
        answer: t(
          'Choose by workload rather than list price. I can compare the live included quota, reset period, and current top-up discounts against your expected usage.'
        ),
        action: {
          kind: 'tool',
          label: t('Compare live plans'),
          tool: 'plan',
        },
      },
      {
        id: 'api-key',
        question: t('What are my Base URL, model ID, and API key?'),
        answer: t(
          'Your Base URL is {{baseUrl}}. Open connection details to copy it, see the current model ID, and create a new API key after explicit confirmation. Existing keys remain private.',
          { baseUrl }
        ),
        action: {
          kind: 'tool',
          label: t('View connection details'),
          tool: 'key',
        },
      },
      {
        id: 'client-setup',
        question: t('How do I set up Claude Code or CC Switch?'),
        answer: t(
          'Windows, Linux, and macOS clients use the same three values: Base URL, model ID, and API key. Open the setup guide for verified install commands and app-specific fields.'
        ),
        action: {
          kind: 'tool',
          label: t('Open client setup guide'),
          tool: 'setup',
        },
        restricted: {
          answer: t(
            'You can install clients while L0 access is under review. API requests become available after L1 approval.'
          ),
          action: {
            kind: 'tool',
            label: t('Open client setup guide'),
            tool: 'setup',
          },
        },
      },
      {
        id: 'bounty',
        question: t('How do open-source bounties and tips work?'),
        answer: t(
          'A publisher funds a challenge and reviews submitted work. When a contribution is accepted, the publisher can add a tip before settlement; every financial confirmation shows the exact amount first.'
        ),
        action: {
          kind: 'route',
          label: t('Explore open-source bounties'),
          to: '/open-source-bounties',
        },
        restricted: {
          answer: t(
            'A publisher funds a challenge and reviews submitted work. When a contribution is accepted, the publisher can add a tip before settlement; every financial confirmation shows the exact amount first.'
          ),
          action: {
            kind: 'route',
            label: t('Explore open-source bounties'),
            to: '/open-source-bounties',
          },
        },
      },
      {
        id: 'cost',
        question: t('How is request cost calculated?'),
        answer: t(
          'Estimated cost is input tokens multiplied by the input rate plus output tokens multiplied by the output rate. Cached tokens, images, or tools may have separate rates, so confirm the selected model in Pricing.'
        ),
        action: {
          kind: 'tool',
          label: t('Calculate with live pricing'),
          tool: 'cost',
        },
        restricted: {
          answer: t(
            'Choose a model and enter expected input and output tokens. I can calculate a read-only estimate from live pricing while your L1 request is under review.'
          ),
          action: {
            kind: 'tool',
            label: t('Calculate with live pricing'),
            tool: 'cost',
          },
        },
      },
      {
        id: 'usage',
        question: t('Can you analyze my historical calls and usage?'),
        answer: t(
          'I can summarize your recent requests, tokens, cost, models, and groups. Ask for a time range such as the last 7 or 30 days, or open the detailed usage logs.'
        ),
        action: {
          kind: 'tool',
          label: t('Open usage statistics'),
          tool: 'usage',
        },
      },
      {
        id: 'models',
        question: t('Which models and model IDs can I use?'),
        answer: t(
          'Ask me for the current model IDs and routing groups. I will read the account-specific list instead of guessing from a public model name.'
        ),
        action: {
          kind: 'tool',
          label: t('View all currently available models'),
          tool: 'models',
        },
      },
      {
        id: 'invitation',
        question: t('How do invitation rewards work?'),
        answer: t(
          'I can show your invitation code, link, invited count, and configured reward amounts. Rewards are calculated from the current account configuration.'
        ),
        action: {
          kind: 'route',
          label: t('Open wallet and invitations'),
          to: '/wallet',
        },
      },
      {
        id: 'human',
        question: t('I need human support'),
        answer: t(
          'Describe the account, billing, or technical issue and include the page and approximate time. Never include an API key or password in the message.'
        ),
        action: {
          kind: 'tool',
          label: t('Send to an administrator'),
          tool: 'handoff',
        },
      },
    ],
    [baseUrl, t]
  )

  const [entries, setEntries] = useState<ConversationEntry[]>(() => {
    const preset = presets.find((item) => item.id === props.initialPreset)
    if (!preset) return []
    return [
      { id: nanoid(), role: 'user', content: preset.question },
      {
        id: nanoid(),
        role: 'assistant',
        content: preset.answer,
        action: preset.action,
      },
    ]
  })
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
  >(props.initialPreset === 'onboarding' ? 'activation' : null)
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
  let visiblePresets: AssistantPreset[] = []
  if (isAdministrator) {
    visiblePresets = presets
  } else if (accountAccessState === 'restricted') {
    visiblePresets = presets.filter((preset) => preset.id === 'onboarding')
  } else if (developerAccessGranted) {
    visiblePresets = presets
  }

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

  const appendPreset = useCallback(
    (preset: AssistantPreset) => {
      const presentation =
        accountAccessState === 'restricted' && preset.restricted
          ? preset.restricted
          : preset
      setActiveTool(preset.id === 'onboarding' ? 'activation' : null)
      setEntries((current) => [
        ...current,
        {
          id: nanoid(),
          role: 'user',
          content: preset.question,
        },
        {
          id: nanoid(),
          role: 'assistant',
          content: presentation.answer,
          action: presentation.action,
        },
      ])
    },
    [accountAccessState]
  )

  useEffect(
    () =>
      subscribeToAssistantOpen((presetId) => {
        if (!presetId) return
        const preset = presets.find((item) => item.id === presetId)
        if (
          accountAccessState !== 'granted' &&
          !(accountAccessState === 'restricted' && preset?.restricted)
        ) {
          return
        }
        if (preset) appendPreset(preset)
      }),
    [accountAccessState, appendPreset, presets]
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
      const suggestedPresetId = getAssistantPresetForIntent(reply.intent)
      const suggestedPreset = suggestedPresetId
        ? presets.find((preset) => preset.id === suggestedPresetId)
        : undefined
      const adminChange =
        reply.action?.type === 'admin_config_change' ||
        reply.action?.type === 'admin_pricing_change'
          ? reply.action
          : undefined
      let suggestedAction = developerAccessGranted
        ? suggestedPreset?.action
        : suggestedPresetId === 'onboarding'
          ? suggestedPreset?.restricted?.action
          : undefined
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
      if (accountAccessState === 'restricted') {
        throw new Error(
          t('Support message must contain at least 5 characters.')
        )
      }
      return
    }
    const validation = getAssistantPromptValidation(
      message,
      accountAccessState === 'restricted'
    )
    if (validation.invalid) {
      throw new Error(t('Support message must contain at least 5 characters.'))
    }
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
        className='m-3 mb-0'
        data-testid='assistant-privacy-notice'
        variant='default'
      >
        <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} aria-hidden='true' />
        <AlertTitle>{t('Conversation privacy notice')}</AlertTitle>
        <AlertDescription>
          {t(
            'Your assistant conversations are not private. Authorized higher-access users may review them; never send personal information, passwords, API keys, or credentials.'
          )}
        </AlertDescription>
      </Alert>
      {historyVisible ? (
        <Conversation className='bg-muted/20'>
          <ConversationContent className='flex min-h-full flex-col gap-5 px-4 py-5 sm:px-6'>
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
          <Conversation className='bg-muted/20'>
            <ConversationContent className='flex min-h-full flex-col gap-5 px-4 py-5 sm:px-6'>
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
                          {t('L0 tutorial required')}
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
                        <Button
                          type='button'
                          className='w-full sm:w-fit'
                          onClick={() => appendPreset(presets[0]!)}
                        >
                          {t('Ask an administrator to raise my access level')}
                          <HugeiconsIcon
                            icon={ArrowRight01Icon}
                            strokeWidth={2}
                            data-icon='inline-end'
                            aria-hidden='true'
                          />
                        </Button>
                      </CardContent>
                    </Card>
                  ) : (
                    <>
                      <div>
                        <p className='text-base font-medium'>
                          {t('How can I help?')}
                        </p>
                        <p className='text-muted-foreground mt-1 text-sm leading-6'>
                          {t(
                            'Choose a common question or ask anything about using LMM.'
                          )}
                        </p>
                      </div>
                      <div className='grid gap-2'>
                        {visiblePresets.map((preset) => (
                          <Button
                            key={preset.id}
                            type='button'
                            variant='outline'
                            className='h-auto min-h-11 justify-between gap-3 px-3 py-2.5 text-left whitespace-normal'
                            onClick={() => appendPreset(preset)}
                          >
                            <span>{preset.question}</span>
                            <HugeiconsIcon
                              icon={ArrowRight01Icon}
                              strokeWidth={2}
                              className='shrink-0'
                              aria-hidden='true'
                            />
                          </Button>
                        ))}
                      </div>
                    </>
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
                            ? 'text-destructive gap-3 text-sm leading-6'
                            : 'gap-3 text-sm leading-6'
                        }
                      >
                        {entry.role === 'assistant' ? (
                          <Response className='leading-7' final>
                            {entry.content}
                          </Response>
                        ) : (
                          <p className='whitespace-pre-wrap'>{entry.content}</p>
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
                              <PresetAction
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
                      onClick={() => {
                        setEntries([])
                        setActiveTool(null)
                        setRecommendationDraft(null)
                        setAccountDisableDraft(null)
                      }}
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

          <div className='bg-background pb-[max(0.75rem,env(safe-area-inset-bottom))]'>
            <Separator className='bg-border/70' />
            <div className='px-3 py-3 sm:px-4'>
              <PromptInputProvider initialInput={props.initialMessage}>
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
        className='bg-background hidden min-h-0 w-[clamp(20rem,28vw,30rem)] shrink-0 flex-col border-l md:flex'
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
        className={sideDrawerContentClassName('max-w-none sm:!max-w-[480px]')}
      >
        {panelContent}
      </SheetContent>
    </Sheet>
  )
}
