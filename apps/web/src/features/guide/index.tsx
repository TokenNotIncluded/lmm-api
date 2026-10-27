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
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import {
  ArrowRight,
  ArrowUpRight,
  BookOpen,
  Check,
  ChevronDown,
  Copy,
  MessageCircle,
  ShieldCheck,
  Terminal,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { getAssistantSetupModels } from '@/features/assistant/api'
import {
  requestAssistantOpen,
  type AssistantPresetId,
} from '@/features/assistant/assistant-events'
import {
  AssistantSetupTool,
  type ClientTab,
} from '@/features/assistant/assistant-setup-tool'
import { ForgePublicShell } from '@/features/forge/forge-public-shell'
import { readSetupPreferences } from '@/features/onboarding/setup-preferences'
import { useStatus } from '@/hooks/use-status'
import {
  getOnboardingState,
  isConsoleActivated,
} from '@/lib/console-activation'
import { useAuthStore } from '@/stores/auth-store'

import { useGuideCopy, type GuideCopy } from './guide-copy'

import './guide.css'

const GUIDE_SECTION_IDS = [
  'client-setup',
  'guide-first-message',
  'guide-troubleshooting',
  'guide-support',
] as const

function GuideContents({ copy }: { copy: GuideCopy }) {
  return (
    <nav aria-label={copy.sectionsTitle} className='guide-contents'>
      <p className='guide-kicker'>{copy.sectionsTitle}</p>
      <ol>
        {GUIDE_SECTION_IDS.map((id, index) => (
          <li key={id}>
            <a href={`#${id}`}>
              <span className='guide-nav-number' aria-hidden='true'>
                {String(index + 1).padStart(2, '0')}
              </span>
              <span>{copy.sections[index]}</span>
              <ArrowUpRight aria-hidden='true' />
            </a>
          </li>
        ))}
      </ol>
    </nav>
  )
}

function GuideCode({
  label,
  value,
  copy,
}: {
  label: string
  value: string
  copy: GuideCopy
}) {
  const [status, setStatus] = useState<'idle' | 'copied' | 'error'>('idle')
  const copyValue = async () => {
    try {
      await navigator.clipboard.writeText(value)
      setStatus('copied')
    } catch {
      setStatus('error')
    }
  }

  return (
    <div className='guide-code'>
      <div className='guide-code-toolbar'>
        <span>{label}</span>
        <Button
          type='button'
          variant='ghost'
          className='guide-copy-button'
          aria-label={`${copy.copy}: ${label}`}
          onClick={() => void copyValue()}
        >
          {status === 'copied' ? (
            <Check aria-hidden='true' />
          ) : (
            <Copy aria-hidden='true' />
          )}
          {status === 'copied' ? copy.copied : copy.copy}
        </Button>
      </div>
      <pre tabIndex={0} aria-label={label}>
        <code>{value}</code>
      </pre>
      <p className='sr-only' role='status'>
        {status === 'copied' ? copy.copied : ''}
      </p>
      {status === 'error' && (
        <p className='guide-copy-error' role='alert'>
          {copy.copyFailed}
        </p>
      )}
    </div>
  )
}

function SectionHeading({
  number,
  label,
  title,
  description,
  id,
}: {
  number: string
  label: string
  title: string
  description: string
  id: string
}) {
  return (
    <div className='guide-section-heading'>
      <p className='guide-kicker'>
        <span aria-hidden='true'>{number} / </span>
        {label}
      </p>
      <h2 id={id}>{title}</h2>
      <p>{description}</p>
    </div>
  )
}

export function Guide() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { status, capabilitiesReady } = useStatus()
  const assistantAvailable =
    capabilitiesReady && status?.assistant?.enabled !== false
  const user = useAuthStore((state) => state.auth.user)
  const copy = useGuideCopy()
  const developerAccessGranted = isConsoleActivated(user)
  const onboarding = getOnboardingState(user)
  const [selectedClient, setSelectedClient] = useState<ClientTab>(
    () => readSetupPreferences()?.client ?? 'cherry-studio'
  )
  const oauth =
    selectedClient === 'pi' ||
    selectedClient === 'dsh' ||
    selectedClient === 'codewhale'
  const completedSteps = oauth
    ? [onboarding.activationComplete, onboarding.firstRequestComplete]
    : [
        onboarding.activationComplete,
        user?.onboarding?.details_available !== false &&
          user?.onboarding?.api_key_created === true,
        onboarding.firstRequestComplete,
      ]
  const accountSteps = oauth
    ? [t('API access'), t('First successful request')]
    : copy.accountSteps
  const stages = oauth
    ? [
        t('Choose your client'),
        selectedClient === 'pi'
          ? t('Install Pi')
          : selectedClient === 'dsh'
            ? t('Install DSH plugin')
            : t('Install Codewhale adapter'),
        t('Sign in with OAuth'),
        t('Choose a model'),
        t('Send first request'),
      ]
    : copy.stages
  const rootUrl =
    typeof window === 'undefined'
      ? 'https://api.lmm.best'
      : window.location.origin
  const modelsQuery = useQuery({
    queryKey: ['guide-available-models', user?.id],
    queryFn: getAssistantSetupModels,
    enabled: developerAccessGranted,
    staleTime: 60_000,
    retry: false,
  })
  const contactSupport = () => {
    if (developerAccessGranted) {
      void navigate({ to: '/support' })
      return
    }
    const support = document.getElementById('guide-support')
    support?.scrollIntoView({ block: 'center' })
    support?.focus({ preventScroll: true })
  }
  const askAssistant = (
    message: string,
    preset: AssistantPresetId = 'client-setup'
  ) => {
    if (!assistantAvailable) {
      contactSupport()
      return
    }
    requestAssistantOpen(preset, message)
    if (user) {
      void navigate({ to: isConsoleActivated(user) ? '/dashboard' : '/wallet' })
    } else {
      void navigate({
        to: '/sign-in',
        search: { redirect: '/dashboard' },
      })
    }
  }
  const createKey = () => {
    if (developerAccessGranted && !assistantAvailable) {
      void navigate({ to: '/keys' })
      return
    }
    askAssistant(copy.createKeyQuestion, 'api-key')
  }
  const requestAccess = () => {
    if (!assistantAvailable && !user) {
      void navigate({ to: '/sign-in', search: { redirect: '/guide' } })
      return
    }
    askAssistant(copy.requestAccessQuestion, 'onboarding')
  }
  const example = String.raw`export LMM_API_KEY='paste-your-key-here'
curl ${rootUrl}/v1/chat/completions \
  -H "Authorization: Bearer $LMM_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"your-model-id","messages":[{"role":"user","content":"Hello"}]}'`
  const stageItems = stages.map((stage, index) => (
    <li key={stage}>
      <span className='guide-stage-number' aria-hidden='true'>
        {String(index + 1).padStart(2, '0')}
      </span>
      <span>{stage}</span>
    </li>
  ))

  return (
    <ForgePublicShell>
      <main className='guide-page'>
        <div className='guide-container'>
          <div className='guide-breadcrumb'>
            <span className='guide-pixel-mark' aria-hidden='true' />
            <span>LMM</span>
            <span aria-hidden='true'>/</span>
            <span>{copy.workspaceLabel}</span>
            <Link to='/developers'>
              {copy.reference}
              <ArrowUpRight aria-hidden='true' />
            </Link>
          </div>

          <header className='guide-hero'>
            <div className='guide-hero-main'>
              <div className='guide-hero-copy'>
                <p className='guide-kicker'>
                  <span className='guide-pixel-dot' aria-hidden='true' />
                  {copy.eyebrow}
                </p>
                <h1>{copy.title}</h1>
                <p className='guide-hero-intro'>
                  {assistantAvailable ? copy.intro : copy.manualIntro}
                </p>
                <div className='guide-actions'>
                  <Button
                    className='guide-primary-action'
                    render={<a href='#client-setup' />}
                  >
                    {copy.start}
                    <ArrowRight aria-hidden='true' />
                  </Button>
                  <Button
                    variant='ghost'
                    className='guide-secondary-action'
                    onClick={
                      assistantAvailable
                        ? () => askAssistant(copy.setupQuestion)
                        : contactSupport
                    }
                  >
                    <MessageCircle aria-hidden='true' />
                    {assistantAvailable
                      ? user
                        ? copy.assistant
                        : copy.guestAssistant
                      : copy.contactSupport}
                  </Button>
                </div>
              </div>
              <div className='guide-connection'>
                <div className='guide-connection-heading'>
                  <Terminal aria-hidden='true' />
                  <span>{oauth ? t('Sign in with OAuth') : copy.endpoint}</span>
                  <span className='guide-connection-mark' aria-hidden='true'>
                    ↗
                  </span>
                </div>
                {oauth ? (
                  <div className='guide-oauth-summary'>
                    <code>
                      {selectedClient === 'pi'
                        ? 'Pi'
                        : selectedClient === 'dsh'
                          ? 'DSH'
                          : 'Codewhale'}{' '}
                      / OAuth
                    </code>
                    <p>
                      {t(
                        'Use Pi or a client that supports LMM OAuth. No manual API key is needed.'
                      )}
                    </p>
                  </div>
                ) : (
                  <>
                    <GuideCode
                      label={t('API endpoint root')}
                      value={rootUrl}
                      copy={copy}
                    />
                    <p className='guide-connection-note'>{copy.endpointHint}</p>
                  </>
                )}
                <Link to='/pricing' className='guide-connection-link'>
                  {copy.browseModels}
                  <ArrowUpRight aria-hidden='true' />
                </Link>
              </div>
            </div>
            <ol className='guide-stages'>{stageItems}</ol>
          </header>

          <details id='guide-overview' className='guide-mobile-contents'>
            <summary>
              {copy.sectionsTitle}
              <ChevronDown aria-hidden='true' />
            </summary>
            <ol className='guide-mobile-stages'>{stageItems}</ol>
            <GuideContents copy={copy} />
          </details>

          <div className='guide-layout'>
            <aside className='guide-sidebar'>
              <GuideContents copy={copy} />
              <div className='guide-sidebar-reference'>
                <BookOpen aria-hidden='true' />
                <Link to='/developers'>
                  {copy.reference}
                  <ArrowUpRight aria-hidden='true' />
                </Link>
              </div>
            </aside>

            <div className='guide-content'>
              <section
                id='client-setup'
                className='guide-section guide-setup-section'
                aria-labelledby='client-setup-title'
              >
                <SectionHeading
                  number='01'
                  label={copy.setupLabel}
                  title={copy.setupTitle}
                  description={copy.setupBody}
                  id='client-setup-title'
                />
                <AssistantSetupTool
                  publicGuide
                  onClientChange={setSelectedClient}
                  rootUrl={rootUrl}
                  openAIBaseUrl={`${rootUrl}/v1`}
                  availableModels={
                    developerAccessGranted ? (modelsQuery.data ?? []) : []
                  }
                  modelsLoading={modelsQuery.isLoading}
                  developerAccessGranted={developerAccessGranted}
                  onCreateKey={createKey}
                  onRequestAccess={requestAccess}
                  onAskQuestion={assistantAvailable ? askAssistant : undefined}
                />
                {developerAccessGranted && modelsQuery.isError && (
                  <div className='guide-model-error' role='alert'>
                    <p>{copy.modelsError}</p>
                    <Button
                      variant='outline'
                      disabled={modelsQuery.isFetching}
                      onClick={() => void modelsQuery.refetch()}
                    >
                      {copy.retry}
                    </Button>
                  </div>
                )}
              </section>

              <section
                id='guide-first-message'
                className='guide-section'
                aria-labelledby='first-message-title'
              >
                <SectionHeading
                  number='02'
                  label={copy.messageLabel}
                  title={copy.firstTitle}
                  description={copy.firstBody}
                  id='first-message-title'
                />
                <div className='guide-first-message'>
                  <GuideCode
                    label={copy.messageLabel}
                    value={copy.firstMessage}
                    copy={copy}
                  />
                </div>
                <div className='guide-inline-links'>
                  {developerAccessGranted && (
                    <Link to='/usage-logs'>
                      {copy.usage}
                      <ArrowUpRight aria-hidden='true' />
                    </Link>
                  )}
                  <Link to='/pricing'>
                    {copy.modelList}
                    <ArrowUpRight aria-hidden='true' />
                  </Link>
                </div>
                {!oauth && (
                  <details className='guide-developer'>
                    <summary>
                      <Terminal aria-hidden='true' />
                      <span>{copy.developerTitle}</span>
                      <ChevronDown aria-hidden='true' />
                    </summary>
                    <p>{copy.developerBody}</p>
                    <GuideCode
                      label='Bash · OpenAI-compatible API'
                      value={example}
                      copy={copy}
                    />
                  </details>
                )}
              </section>

              <section
                id='guide-troubleshooting'
                className='guide-section'
                aria-labelledby='troubleshooting-title'
              >
                <SectionHeading
                  number='03'
                  label={copy.troubleshootLabel}
                  title={copy.troubleTitle}
                  description={
                    assistantAvailable
                      ? copy.troubleBody
                      : copy.manualTroubleBody
                  }
                  id='troubleshooting-title'
                />
                <div className='guide-errors'>
                  {copy.troubles.map((trouble) => (
                    <details key={trouble.code}>
                      <summary>
                        <code>{trouble.code}</code>
                        <span>{trouble.title}</span>
                        <ChevronDown aria-hidden='true' />
                      </summary>
                      <div className='guide-error-content'>
                        <p>{trouble.body}</p>
                        <Button
                          variant='outline'
                          onClick={
                            assistantAvailable
                              ? () => askAssistant(trouble.question)
                              : contactSupport
                          }
                        >
                          <MessageCircle aria-hidden='true' />
                          {assistantAvailable
                            ? user
                              ? copy.askAboutError
                              : copy.guestError
                            : copy.contactSupport}
                        </Button>
                      </div>
                    </details>
                  ))}
                </div>
              </section>

              <section
                id='guide-support'
                tabIndex={-1}
                className='guide-section guide-support'
                aria-labelledby='guide-support-title'
              >
                <SectionHeading
                  number='04'
                  label={copy.supportLabel}
                  title={copy.accountTitle}
                  description={
                    oauth
                      ? t(
                          'Use Pi or a client that supports LMM OAuth. No manual API key is needed.'
                        )
                      : copy.accountBody
                  }
                  id='guide-support-title'
                />
                <div className='guide-support-grid'>
                  <div className='guide-checklist'>
                    <h3 className='guide-kicker'>{copy.checklistLabel}</h3>
                    <ol>
                      {accountSteps.map((step, index) => (
                        <li key={step}>
                          <span
                            className='guide-check-state'
                            data-complete={completedSteps[index]}
                            aria-label={
                              completedSteps[index]
                                ? copy.complete
                                : copy.pending
                            }
                          >
                            {completedSteps[index] ? (
                              <Check aria-hidden='true' />
                            ) : (
                              <span aria-hidden='true'>{index + 1}</span>
                            )}
                          </span>
                          <span>{step}</span>
                        </li>
                      ))}
                    </ol>
                    {oauth && developerAccessGranted ? (
                      <Button render={<a href='#client-setup' />}>
                        {t('Continue client setup')}
                        <ArrowRight aria-hidden='true' />
                      </Button>
                    ) : user ? (
                      <Button
                        onClick={
                          developerAccessGranted ? createKey : requestAccess
                        }
                      >
                        {developerAccessGranted
                          ? copy.createKey
                          : assistantAvailable
                            ? copy.requestAccess
                            : copy.requestManualAccess}
                        <ArrowRight aria-hidden='true' />
                      </Button>
                    ) : (
                      <Button
                        render={
                          <Link to='/sign-in' search={{ redirect: '/guide' }} />
                        }
                      >
                        {copy.signIn}
                        <ArrowRight aria-hidden='true' />
                      </Button>
                    )}
                  </div>
                  <div className='guide-human-support'>
                    <MessageCircle aria-hidden='true' />
                    <h3>{copy.supportTitle}</h3>
                    <p>{copy.supportBody}</p>
                    <a href='mailto:support@lmm.best'>
                      support@lmm.best
                      <ArrowUpRight aria-hidden='true' />
                    </a>
                    {developerAccessGranted && (
                      <Link to='/support'>
                        {copy.supportTicket}
                        <ArrowUpRight aria-hidden='true' />
                      </Link>
                    )}
                  </div>
                </div>
                {!oauth && (
                  <div className='guide-security-note'>
                    <ShieldCheck aria-hidden='true' />
                    <h3>{copy.securityTitle}</h3>
                    <p>{copy.securityBody}</p>
                  </div>
                )}
              </section>
              <footer className='guide-footer'>
                <span>LMM API</span>
                <Link to='/developers'>
                  {copy.reference}
                  <ArrowUpRight aria-hidden='true' />
                </Link>
              </footer>
            </div>
          </div>
        </div>
      </main>
    </ForgePublicShell>
  )
}
