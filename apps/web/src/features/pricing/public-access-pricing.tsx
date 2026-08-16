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
  ArrowRight01Icon,
  ChartIncreaseIcon,
  WalletCardsIcon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { useStatus } from '@/hooks/use-status'
import { isLocalPreview } from '@/lib/local-preview'

import { canOfferRegistration } from '../auth/lib/registration'
import { ForgePublicShell } from '../forge/forge-public-shell'

export function PublicAccessPricing() {
  const { t } = useTranslation()
  const { status, capabilitiesReady } = useStatus()
  const canRegister = canOfferRegistration(
    status,
    capabilitiesReady,
    isLocalPreview()
  )
  const accountPath = canRegister ? '/sign-up' : '/sign-in'
  const accountLabel = canRegister ? t('Create account') : t('Sign in')

  return (
    <ForgePublicShell>
      <main>
        <section className='border-border bg-background border-b pt-28 pb-16 md:pt-36 md:pb-24'>
          <div className='mx-auto max-w-7xl px-5 md:px-10'>
            <div className='max-w-3xl'>
              <Badge
                variant='outline'
                className='border-border mb-5 rounded-sm'
              >
                {t('Pay as you go')}
              </Badge>
              <h1 className='max-w-3xl font-serif text-5xl leading-[1.02] font-normal md:text-7xl'>
                {t('Developer access that grows with your work')}
              </h1>
              <p className='mt-7 max-w-2xl text-base leading-7 md:text-lg'>
                {t(
                  'Create an account, add usage credit when you are ready, and pay only for what you use.'
                )}
              </p>
              <div className='mt-8 flex flex-col gap-3 sm:flex-row'>
                <Button
                  size='lg'
                  className='rounded-sm'
                  render={<Link to={accountPath} />}
                >
                  {accountLabel}
                  {canRegister ? (
                    <HugeiconsIcon
                      icon={ArrowRight01Icon}
                      data-icon='inline-end'
                      strokeWidth={2}
                      aria-hidden='true'
                    />
                  ) : null}
                </Button>
                <Button
                  size='lg'
                  variant='outline'
                  className='rounded-sm'
                  render={<Link to='/sign-in' />}
                >
                  {t('Sign in')}
                </Button>
              </div>
            </div>
          </div>
        </section>

        <section className='border-border bg-muted/60 border-b py-12 md:py-16'>
          <div className='mx-auto grid max-w-7xl gap-4 px-5 sm:grid-cols-3 md:px-10'>
            <div className='flex items-start gap-3'>
              <HugeiconsIcon
                icon={WalletCardsIcon}
                className='mt-0.5 size-6 shrink-0'
                strokeWidth={2}
                aria-hidden='true'
              />
              <div>
                <p className='font-medium'>{t('No monthly commitment')}</p>
                <p className='text-muted-foreground mt-1 text-sm leading-6'>
                  {t('Add credit when a project needs it.')}
                </p>
              </div>
            </div>
            <div className='flex items-start gap-3'>
              <HugeiconsIcon
                icon={ChartIncreaseIcon}
                className='mt-0.5 size-6 shrink-0'
                strokeWidth={2}
                aria-hidden='true'
              />
              <div>
                <p className='font-medium'>{t('Trust and volume benefits')}</p>
                <p className='text-muted-foreground mt-1 text-sm leading-6'>
                  {t(
                    'Consistent, verified activity can improve value over time.'
                  )}
                </p>
              </div>
            </div>
            <div className='flex items-start gap-3'>
              <HugeiconsIcon
                icon={WalletCardsIcon}
                className='mt-0.5 size-6 shrink-0'
                strokeWidth={2}
                aria-hidden='true'
              />
              <div>
                <p className='font-medium'>{t('Top up after sign-up')}</p>
                <p className='text-muted-foreground mt-1 text-sm leading-6'>
                  {t('Review the available options inside your workspace.')}
                </p>
              </div>
            </div>
          </div>
        </section>

        <section className='border-border border-b py-16 md:py-24'>
          <div className='mx-auto max-w-7xl px-5 md:px-10'>
            <div className='mb-10 max-w-2xl'>
              <p className='mb-3 text-xs font-bold uppercase'>
                {t('Choose your starting point')}
              </p>
              <h2 className='font-serif text-4xl leading-tight font-normal md:text-5xl'>
                {t('Three ways to begin')}
              </h2>
              <p className='text-muted-foreground mt-4 text-sm leading-6 md:text-base'>
                {t(
                  'Each option uses the same pay-as-you-go balance. Start with the level of commitment that matches your work.'
                )}
              </p>
            </div>

            <div className='grid gap-4 lg:grid-cols-3'>
              <Card className='border-border border'>
                <CardHeader>
                  <CardTitle className='font-serif text-2xl font-normal'>
                    {t('Start small')}
                  </CardTitle>
                  <CardDescription>
                    {t('For trying the service')}
                  </CardDescription>
                </CardHeader>
                <CardContent className='flex-1 text-sm leading-6'>
                  {t(
                    'Create your account, add a small balance, and begin when you are ready.'
                  )}
                </CardContent>
                <CardFooter>
                  <Button
                    variant='outline'
                    className='w-full rounded-sm'
                    render={<Link to={accountPath} />}
                  >
                    {accountLabel}
                  </Button>
                </CardFooter>
              </Card>

              <Card className='border-primary bg-accent text-accent-foreground border'>
                <CardHeader>
                  <CardTitle className='font-serif text-2xl font-normal'>
                    {t('Ongoing work')}
                  </CardTitle>
                  <CardDescription className='text-accent-foreground/75'>
                    {t('For regular development')}
                  </CardDescription>
                </CardHeader>
                <CardContent className='flex-1 text-sm leading-6'>
                  {t(
                    'Keep a reusable balance and add more credit as your work grows.'
                  )}
                </CardContent>
                <CardFooter className='bg-muted/60'>
                  <Button
                    className='w-full rounded-sm'
                    render={<Link to={accountPath} />}
                  >
                    {accountLabel}
                  </Button>
                </CardFooter>
              </Card>

              <Card className='border-border border'>
                <CardHeader>
                  <CardTitle className='font-serif text-2xl font-normal'>
                    {t('Higher volume')}
                  </CardTitle>
                  <CardDescription>{t('For sustained usage')}</CardDescription>
                </CardHeader>
                <CardContent className='flex-1 text-sm leading-6'>
                  {t(
                    'Verified top-ups can raise your trust level and improve value over time.'
                  )}
                </CardContent>
                <CardFooter>
                  <Button
                    variant='outline'
                    className='w-full rounded-sm'
                    render={<Link to={accountPath} />}
                  >
                    {accountLabel}
                  </Button>
                </CardFooter>
              </Card>
            </div>

            <p className='text-muted-foreground mt-8 text-sm leading-6'>
              {t(
                'Current usage rates and available amounts are shown after access is activated.'
              )}
            </p>
          </div>
        </section>
      </main>
    </ForgePublicShell>
  )
}
