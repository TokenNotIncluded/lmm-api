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
import { Link, useNavigate } from '@tanstack/react-router'
import { type FormEvent, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useAuthStore } from '@/stores/auth-store'

import { ChallengeList } from './challenge-list'
import { ForgePublicShell } from './forge-public-shell'

export function ForgeHome() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const user = useAuthStore((state) => state.auth.user)
  const [message, setMessage] = useState('')

  const submitMessage = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!message.trim()) return
    void navigate({ to: user ? '/dashboard' : '/sign-in' })
  }

  return (
    <ForgePublicShell minimalNav>
      <main>
        <section
          aria-labelledby='forge-home-title'
          className='border-border border-b'
        >
          <div className='mx-auto grid min-h-[min(46rem,calc(100svh-5rem))] max-w-5xl content-center gap-12 px-5 py-16 md:px-10 md:py-24'>
            <div className='max-w-3xl'>
              <p className='text-muted-foreground mb-5 text-sm'>
                {t('Developer-friendly AI gateway')}
              </p>
              <h1
                id='forge-home-title'
                className='mb-7 max-w-3xl font-serif text-5xl leading-[1.02] font-normal tracking-tight md:text-7xl'
              >
                LMM Forge
              </h1>
              <p className='text-muted-foreground max-w-2xl text-lg leading-8 md:text-xl'>
                {t(
                  'A semi-public-interest AI gateway for high-quality, transparent access.'
                )}
              </p>
              <p className='text-muted-foreground mt-3 max-w-2xl leading-7'>
                {t(
                  'Use the gateway for your own work, or browse public open-source challenges.'
                )}
              </p>
            </div>

            <form
              className='border-border bg-muted/20 grid max-w-2xl gap-3 border p-3 sm:grid-cols-[1fr_auto] sm:items-center'
              onSubmit={submitMessage}
            >
              <label className='sr-only' htmlFor='forge-home-message'>
                {t('Tell us what you want to do')}
              </label>
              <input
                id='forge-home-message'
                value={message}
                onChange={(event) => setMessage(event.target.value)}
                className='bg-background border-border h-11 min-w-0 border px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-current'
                placeholder={t('Describe what you need...')}
                maxLength={4000}
              />
              <Button
                type='submit'
                className='h-11 rounded-sm'
                disabled={!message.trim()}
              >
                {t('Sign in or create an account to continue')}
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  data-icon='inline-end'
                  strokeWidth={2}
                  aria-hidden='true'
                />
              </Button>
            </form>
          </div>
        </section>

        <section
          aria-labelledby='forge-public-challenges-title'
          className='border-border border-b'
        >
          <div className='mx-auto max-w-5xl px-5 py-12 md:px-10 md:py-16'>
            <div className='mb-6 flex items-end justify-between gap-4'>
              <div>
                <p className='text-muted-foreground mb-2 text-sm'>
                  {t('Public challenges')}
                </p>
                <h2
                  id='forge-public-challenges-title'
                  className='font-serif text-3xl font-normal md:text-4xl'
                >
                  {t('Open-source challenges')}
                </h2>
              </div>
              <Button
                variant='outline'
                className='shrink-0 rounded-sm'
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
            <p className='text-muted-foreground mb-5 text-sm'>
              {t('The public board is open to everyone.')}
            </p>
            <ChallengeList limit={3} showHeading={false} />
          </div>
        </section>
      </main>
    </ForgePublicShell>
  )
}
