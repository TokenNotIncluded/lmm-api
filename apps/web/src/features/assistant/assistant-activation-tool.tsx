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
/*
Copyright (C) 2026 LIghtJUNction
*/
import {
  ArrowRight01Icon,
  CheckmarkCircle02Icon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import {
  getDeveloperAccessRequest,
  submitDeveloperAccessRequest,
  type DeveloperAccessRequest,
} from '@/features/onboarding/api'

import type { AssistantL1RecommendationAction } from './api'

function AdministratorReply(props: { request: DeveloperAccessRequest }) {
  const { t } = useTranslation()
  if (!props.request.admin_note) return null
  return (
    <div className='border-border/70 bg-background/70 grid gap-1 border p-3'>
      <p className='text-xs font-medium'>{t('Administrator reply')}</p>
      <p className='text-muted-foreground text-xs leading-5 whitespace-pre-wrap'>
        {props.request.admin_note}
      </p>
    </div>
  )
}

export function AssistantActivationTool(props: {
  recommendationDraft?: AssistantL1RecommendationAction | null
  onContinueSetup?: () => void
  onSubmitted?: (request: DeveloperAccessRequest) => void
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [requestOverride, setRequestOverride] =
    useState<DeveloperAccessRequest | null>(null)
  const [loading, setLoading] = useState(false)
  const [manualReason, setManualReason] = useState('')
  const [letter, setLetter] = useState('')

  const requestQuery = useQuery({
    queryKey: ['assistant-developer-access-request'],
    queryFn: getDeveloperAccessRequest,
    staleTime: 0,
    retry: false,
    refetchInterval: (query) =>
      query.state.data?.status === 'pending' ? 15_000 : false,
  })
  const request = requestOverride ?? requestQuery.data ?? null

  useEffect(() => {
    if (request?.status === 'approved') {
      void queryClient.invalidateQueries({ queryKey: ['assistant-status'] })
    }
  }, [queryClient, request?.status])

  const submit = async () => {
    const draft = props.recommendationDraft
    if (loading || request?.status === 'pending' || !draft) return
    const reason = manualReason.trim() || draft.user_statement
    const recommendation = letter.trim() || draft.recommendation
    setLoading(true)
    try {
      const submitted = await submitDeveloperAccessRequest({
        reason,
        ai_recommendation: recommendation,
        confirmation_token: draft.confirmation_token,
        confirmed: true,
      })
      setRequestOverride(submitted)
      props.onSubmitted?.(submitted)
      toast.success(t('Unlock request submitted'))
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to submit unlock request')
      )
    } finally {
      setLoading(false)
    }
  }

  const savePendingLetter = async () => {
    if (!request || request.status !== 'pending' || loading) return
    const recommendation =
      letter.trim() ||
      props.recommendationDraft?.recommendation ||
      request.ai_recommendation ||
      request.reason
    const reason = props.recommendationDraft?.user_statement || recommendation
    if (recommendation.length < 20) return
    setLoading(true)
    try {
      const submitted = await submitDeveloperAccessRequest({
        reason,
        ai_recommendation: recommendation,
        confirmation_token: props.recommendationDraft?.confirmation_token,
        confirmed: true,
      })
      setRequestOverride(submitted)
      setLetter(submitted.ai_recommendation)
      toast.success(t('Your changes were saved.'))
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to submit unlock request')
      )
    } finally {
      setLoading(false)
    }
  }

  const submitWithoutAI = async () => {
    const reason = manualReason.trim()
    if (loading || props.recommendationDraft || reason.length < 5) return
    setLoading(true)
    try {
      const submitted = await submitDeveloperAccessRequest({
        reason,
        confirmed: true,
      })
      setRequestOverride(submitted)
      props.onSubmitted?.(submitted)
      toast.success(t('Unlock request submitted'))
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to submit unlock request')
      )
    } finally {
      setLoading(false)
    }
  }

  if (request?.status === 'approved') {
    return (
      <section className='border-success/40 bg-success/5 grid gap-3 border p-4'>
        <h3 className='flex items-center gap-2 text-sm font-medium'>
          <HugeiconsIcon
            icon={CheckmarkCircle02Icon}
            className='text-success size-4'
            strokeWidth={2}
            aria-hidden='true'
          />
          {t('L1 access approved')}
        </h3>
        <p className='text-muted-foreground text-sm leading-6'>
          {t(
            'Your developer access is active. Continue setup to create a key and connect your client.'
          )}
        </p>
        <AdministratorReply request={request} />
        {props.onContinueSetup ? (
          <Button type='button' onClick={props.onContinueSetup}>
            {t('Continue setup')}
            <HugeiconsIcon
              icon={ArrowRight01Icon}
              strokeWidth={2}
              data-icon='inline-end'
              aria-hidden='true'
            />
          </Button>
        ) : null}
      </section>
    )
  }

  if (request?.status === 'pending') {
    const recommendation =
      letter ||
      props.recommendationDraft?.recommendation ||
      request.ai_recommendation ||
      request.reason
    return (
      <section className='grid gap-3 border p-4'>
        <div>
          <h3 className='text-sm font-medium'>{t('Recommendation letter')}</h3>
          <p className='text-muted-foreground mt-1 text-xs leading-5'>
            {t(
              'AI and you can edit the same letter before an administrator reviews it.'
            )}
          </p>
        </div>
        <Textarea
          value={recommendation}
          onChange={(event) => setLetter(event.target.value)}
          maxLength={2000}
          rows={5}
          placeholder={t('Recommendation letter')}
          aria-label={t('Recommendation letter')}
        />
        <AdministratorReply request={request} />
        <Button
          type='button'
          size='sm'
          onClick={() => void savePendingLetter()}
          disabled={loading || recommendation.trim().length < 20}
        >
          {loading ? t('Submitting...') : t('Save changes')}
        </Button>
      </section>
    )
  }

  const draft = props.recommendationDraft

  return (
    <section className='grid gap-3 border p-4'>
      <h3 className='text-sm font-medium'>
        {draft ? t('Confirm AI recommendation') : t('Unlock L1 with AI')}
      </h3>
      <p className='text-muted-foreground text-xs leading-5'>
        {draft
          ? t(
              'Review what the AI will send. Nothing is submitted until you explicitly confirm.'
            )
          : t(
              'Continue chatting and explain your use case. When the AI has enough information, it will prepare a recommendation for your confirmation.'
            )}
      </p>
      {request?.status === 'rejected' ? (
        <div className='border-destructive/40 bg-destructive/5 grid gap-2 border p-3'>
          <p className='text-sm font-medium'>
            {t('Previous request rejected')}
          </p>
          <AdministratorReply request={request} />
          {!draft ? (
            <p className='text-muted-foreground text-xs leading-5'>
              {t(
                'Continue the conversation and address the administrator feedback before asking the AI to prepare another recommendation.'
              )}
            </p>
          ) : null}
        </div>
      ) : null}
      {draft ? (
        <div className='grid gap-3'>
          <Textarea
            value={letter || draft.recommendation}
            onChange={(event) => setLetter(event.target.value)}
            maxLength={2000}
            rows={5}
            aria-label={t('Recommendation letter')}
          />
          <Button
            type='button'
            onClick={() => void submit()}
            disabled={loading}
          >
            {loading
              ? t('Submitting...')
              : t('Confirm and send to administrator')}
          </Button>
        </div>
      ) : (
        <div className='grid gap-3'>
          <Textarea
            value={manualReason}
            onChange={(event) => setManualReason(event.target.value)}
            placeholder={t(
              'Explain what you want to build and why you need L1 access.'
            )}
            maxLength={2000}
            rows={4}
            aria-label={t('L1 access request explanation')}
            disabled={loading}
          />
          <p className='text-muted-foreground text-xs leading-5'>
            {t(
              'You can submit for administrator review without an AI recommendation. The recommendation only gives the reviewer more context; it never decides access.'
            )}
          </p>
          <Button
            type='button'
            variant='outline'
            onClick={() => void submitWithoutAI()}
            disabled={loading || manualReason.trim().length < 5}
          >
            {loading
              ? t('Submitting...')
              : t('Submit for administrator review')}
          </Button>
        </div>
      )}
    </section>
  )
}
