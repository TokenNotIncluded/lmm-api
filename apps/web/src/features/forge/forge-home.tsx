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
import { ArrowRight01Icon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { type FormEvent, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '@/components/ui/input-group'
import { getAssistantPreConversationPresets } from '@/features/assistant/api'
import { requestAssistantSend } from '@/features/assistant/assistant-events'
import { redactAssistantMessageForRequest } from '@/features/assistant/assistant-message-safety'
import { getAssistantPromptValidation } from '@/features/assistant/assistant-prompt-validation'
import { useStatus } from '@/hooks/use-status'
import { isConsoleActivated } from '@/lib/console-activation'
import { useAuthStore } from '@/stores/auth-store'

import { ChallengeList } from './challenge-list'
import { ForgePublicShell } from './forge-public-shell'
import { useTypewriterPlaceholder } from './use-typewriter-placeholder'

export function ForgeHome() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const user = useAuthStore((state) => state.auth.user)
  const { status } = useStatus()
  const [message, setMessage] = useState('')
  const [messageFocused, setMessageFocused] = useState(false)
  const assistantEnabled = status?.assistant?.enabled !== false
  const messageInvalid = getAssistantPromptValidation(message).invalid
  const preConversationPresetsQuery = useQuery({
    queryKey: ['assistant-pre-conversation-presets'],
    queryFn: getAssistantPreConversationPresets,
    enabled: assistantEnabled,
    staleTime: 5 * 60_000,
    retry: false,
  })
  const animatedPlaceholder = useTypewriterPlaceholder(
    preConversationPresetsQuery.data?.presets.map((preset) => preset.prompt) ??
      [],
    message.length === 0 && !messageFocused
  )

  const submitMessage = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const safeMessage = redactAssistantMessageForRequest(message).content.trim()
    if (!safeMessage || messageInvalid || !assistantEnabled) return

    if (!user) {
      requestAssistantSend(undefined, safeMessage)
      void navigate({
        to: '/sign-in',
        // The route guard redirects L0 to getting-started after login while
        // L1+ stays on the dashboard.
        search: { redirect: '/dashboard' },
      })
      return
    }

    const activated = isConsoleActivated(user)
    requestAssistantSend(activated ? 'service' : 'onboarding', safeMessage)
    void navigate({ to: activated ? '/dashboard' : '/getting-started' })
  }

  return (
    <ForgePublicShell>
      <main>
        <section
          aria-labelledby='forge-home-title'
          className='forge-home-hero border-border border-b'
        >
          <div className='mx-auto grid max-w-6xl gap-14 px-6 py-20 md:min-h-[min(43rem,calc(100dvh-5rem))] md:grid-cols-[minmax(0,1.2fr)_minmax(18rem,0.7fr)] md:items-center md:px-10 md:py-24'>
            <div className='max-w-3xl'>
              <p className='forge-kicker mb-6'>
                {t('Developer-friendly AI gateway')}
              </p>
              <h1
                id='forge-home-title'
                className='mb-8 max-w-3xl font-serif text-5xl leading-[0.98] font-normal tracking-[-0.035em] sm:text-6xl md:text-8xl'
              >
                LMM Forge
              </h1>
              <p className='text-foreground/70 max-w-2xl text-lg leading-8 sm:text-xl'>
                {t(
                  'A semi-public-interest AI gateway for high-quality, transparent access.'
                )}
              </p>
              <p className='text-muted-foreground mt-4 max-w-xl leading-7'>
                {t(
                  'Use one clear API for your work, connect a client, or explore public open-source challenges.'
                )}
              </p>

              <form className='mt-12 max-w-2xl' onSubmit={submitMessage}>
                <label className='sr-only' htmlFor='forge-home-message'>
                  {t('Tell us what you want to do')}
                </label>
                <InputGroup className='has-[[data-slot=input-group-control]:focus-visible]:border-foreground/50 h-12 rounded-xl has-[[data-slot=input-group-control]:focus-visible]:ring-0'>
                  <InputGroupInput
                    id='forge-home-message'
                    value={message}
                    onChange={(event) => setMessage(event.target.value)}
                    onFocus={() => setMessageFocused(true)}
                    onBlur={() => setMessageFocused(false)}
                    className='focus-visible:!outline-none'
                    placeholder={
                      animatedPlaceholder || t('Describe what you need...')
                    }
                    maxLength={4000}
                  />
                  <InputGroupAddon align='inline-end' className='pr-1'>
                    <InputGroupButton
                      type='submit'
                      variant='default'
                      size='sm'
                      className='h-10 rounded-lg px-3'
                      disabled={
                        !message.trim() || messageInvalid || !assistantEnabled
                      }
                    >
                      <span className='forge-home-submit-label'>
                        {t('Ask AI assistant')}
                      </span>
                      <HugeiconsIcon
                        icon={ArrowRight01Icon}
                        data-icon='inline-end'
                        strokeWidth={2}
                        aria-hidden='true'
                      />
                    </InputGroupButton>
                  </InputGroupAddon>
                </InputGroup>
              </form>
              <div className='mt-5 flex flex-wrap gap-x-6 gap-y-2 text-sm'>
                <Link className='forge-text-link' to='/pricing'>
                  {t('View model pricing')}
                </Link>
                <Link className='forge-text-link' to='/challenges'>
                  {t('Browse open-source work')}
                </Link>
                <Link className='forge-text-link' to='/guide'>
                  {t('Read the guide')}
                </Link>
              </div>
            </div>

            <aside className='forge-home-note' aria-label={t('At a glance')}>
              <div className='forge-note-rule' />
              <p className='forge-kicker mb-5'>{t('At a glance')}</p>
              <ul className='space-y-5 text-sm leading-6'>
                <li>
                  <strong className='font-medium'>{t('One endpoint')}</strong>
                  <span className='text-muted-foreground mt-1 block'>
                    {t('OpenAI and Anthropic-compatible routes.')}
                  </span>
                </li>
                <li>
                  <strong className='font-medium'>{t('Clear pricing')}</strong>
                  <span className='text-muted-foreground mt-1 block'>
                    {t('Choose the model and group before you spend.')}
                  </span>
                </li>
                <li>
                  <strong className='font-medium'>{t('Human review')}</strong>
                  <span className='text-muted-foreground mt-1 block'>
                    {t('Support and access requests stay auditable.')}
                  </span>
                </li>
              </ul>
            </aside>
          </div>
        </section>

        <section
          aria-labelledby='forge-public-challenges-title'
          className='border-border border-b'
        >
          <div className='mx-auto max-w-6xl px-6 py-16 md:px-10 md:py-20'>
            <div className='mb-8 flex items-end justify-between gap-4'>
              <div>
                <p className='text-muted-foreground mb-2 text-sm'>
                  {t('Public challenges')}
                </p>
                <h2
                  id='forge-public-challenges-title'
                  className='font-serif text-3xl font-normal tracking-[-0.025em] md:text-4xl'
                >
                  {t('Open-source challenges')}
                </h2>
              </div>
              <Button
                variant='outline'
                className='forge-outline-button shrink-0 rounded-sm'
                render={<Link to='/challenges' />}
              >
                {t('Browse challenges')}
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  data-icon='inline-end'
                  strokeWidth={2}
                  aria-hidden='true'
                />
              </Button>
            </div>
            <p className='text-muted-foreground mb-7 max-w-xl text-sm leading-6'>
              {t('The public board is open to everyone.')}
            </p>
            <ChallengeList limit={3} showHeading={false} />
          </div>
        </section>
      </main>
    </ForgePublicShell>
  )
}
