/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/

import { Key01Icon, ShieldUserIcon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'

import {
  getOAuthGrantSummaries,
  revokeAllOAuthGrants,
  revokeOAuthGrant,
  type OAuthGrantSummary,
} from '../api'

const grantsQueryKey = ['profile', 'oauth-grants'] as const

function formatDate(value: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value * 1000))
}

function clientLabel(grant: OAuthGrantSummary, t: (value: string) => string) {
  return grant.client_id === 'lmm-pi'
    ? t('Model client')
    : t('Read-only client')
}

export function OAuthGrantsCard() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [revokeTarget, setRevokeTarget] = useState<OAuthGrantSummary | null>(
    null
  )
  const [revokeAllOpen, setRevokeAllOpen] = useState(false)
  const grantsQuery = useQuery({
    queryKey: grantsQueryKey,
    queryFn: getOAuthGrantSummaries,
  })
  const revokeMutation = useMutation({
    mutationFn: revokeOAuthGrant,
    onSuccess: async (response) => {
      if (!response.success) {
        throw new Error(response.message || t('Unable to revoke access'))
      }
      setRevokeTarget(null)
      toast.success(t('Model access revoked'))
      await queryClient.invalidateQueries({ queryKey: grantsQueryKey })
    },
    onError: (error: Error) => toast.error(error.message),
  })
  const revokeAllMutation = useMutation({
    mutationFn: revokeAllOAuthGrants,
    onSuccess: async (response) => {
      if (!response.success) {
        throw new Error(response.message || t('Unable to revoke access'))
      }
      setRevokeAllOpen(false)
      toast.success(t('All model access was revoked'))
      await queryClient.invalidateQueries({ queryKey: grantsQueryKey })
    },
    onError: (error: Error) => toast.error(error.message),
  })

  const grants = grantsQuery.data?.data ?? []
  const active = grants.filter((grant) => grant.active)

  return (
    <>
      <Card data-card-hover='false'>
        <CardHeader>
          <CardTitle className='flex items-center gap-2'>
            <HugeiconsIcon icon={ShieldUserIcon} strokeWidth={2} />
            {t('Authorized clients')}
          </CardTitle>
          <CardDescription>
            {t(
              'Review applications that can read your account or call models. Revoking access takes effect immediately.'
            )}
          </CardDescription>
          <CardAction>
            <Button
              type='button'
              variant='destructive'
              size='sm'
              disabled={active.length === 0 || revokeAllMutation.isPending}
              onClick={() => setRevokeAllOpen(true)}
            >
              {t('Revoke all access')}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent>
          {grantsQuery.isLoading ? (
            <div className='space-y-3'>
              <Skeleton className='h-24 w-full' />
              <Skeleton className='h-24 w-full' />
            </div>
          ) : grantsQuery.isError ? (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant='icon'>
                  <HugeiconsIcon icon={Key01Icon} strokeWidth={2} />
                </EmptyMedia>
                <EmptyTitle>
                  {t('Unable to load authorized clients')}
                </EmptyTitle>
                <EmptyDescription>
                  {t('Refresh the page and try again.')}
                </EmptyDescription>
              </EmptyHeader>
              <Button
                type='button'
                variant='outline'
                onClick={() => grantsQuery.refetch()}
              >
                {t('Retry')}
              </Button>
            </Empty>
          ) : grants.length === 0 ? (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant='icon'>
                  <HugeiconsIcon icon={Key01Icon} strokeWidth={2} />
                </EmptyMedia>
                <EmptyTitle>{t('No authorized clients')}</EmptyTitle>
                <EmptyDescription>
                  {t('Applications will appear here after you approve access.')}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            <div className='space-y-3'>
              {grants.map((grant) => (
                <div
                  key={grant.id}
                  className='border-border/70 rounded-lg border p-4'
                >
                  <div className='flex flex-wrap items-start justify-between gap-3'>
                    <div className='space-y-1'>
                      <p className='font-medium'>{clientLabel(grant, t)}</p>
                      <p className='text-muted-foreground text-sm'>
                        {grant.purpose === 'normal_model'
                          ? t('Model access')
                          : t('Read-only account access')}
                      </p>
                    </div>
                    {grant.active ? (
                      <Button
                        type='button'
                        variant='outline'
                        size='sm'
                        disabled={revokeMutation.isPending}
                        onClick={() => setRevokeTarget(grant)}
                      >
                        {t('Revoke')}
                      </Button>
                    ) : (
                      <Badge variant='secondary'>
                        {grant.revoked_at ? t('Revoked') : t('Expired')}
                      </Badge>
                    )}
                  </div>
                  <div className='mt-3 flex flex-wrap gap-2'>
                    {grant.scopes.map((scope) => (
                      <Badge key={scope} variant='outline'>
                        {scope}
                      </Badge>
                    ))}
                  </div>
                  <p className='text-muted-foreground mt-3 text-xs'>
                    {grant.budget_credits
                      ? `${t('Budget')}: ${grant.budget_credits.toLocaleString()} ${t('credits')} · `
                      : ''}
                    {grant.active
                      ? `${t('Expires')}: ${formatDate(grant.expires_at)}`
                      : grant.revoked_at
                        ? `${t('Revoked')}: ${formatDate(grant.revoked_at)}`
                        : `${t('Expired')}: ${formatDate(grant.expires_at)}`}
                  </p>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
      <ConfirmDialog
        open={revokeTarget !== null}
        onOpenChange={(open) => !open && setRevokeTarget(null)}
        title={t('Revoke model access?')}
        desc={t(
          'The client will stop receiving model authorization immediately. You can authorize it again later.'
        )}
        destructive
        confirmText={t('Revoke access')}
        isLoading={revokeMutation.isPending}
        handleConfirm={() => {
          if (revokeTarget) revokeMutation.mutate(revokeTarget.id)
        }}
      />
      <ConfirmDialog
        open={revokeAllOpen}
        onOpenChange={setRevokeAllOpen}
        title={t('Revoke all authorized clients?')}
        desc={t(
          'Every OAuth client shown here will lose access immediately, including model access.'
        )}
        destructive
        confirmText={t('Revoke all')}
        isLoading={revokeAllMutation.isPending}
        handleConfirm={() => revokeAllMutation.mutate()}
      />
    </>
  )
}
